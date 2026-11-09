//! Rust-local imports resolved against syntax and module declarations, never compiler execution.
//! Workspace crates declared as local `[dependencies]` link to their defining file; registry
//! crates, macro expansion and cfg evaluation are intentionally outside this index.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use archguard_core::{ArchguardError, Ref};
use sha2::{Digest, Sha256};
use syn::{Item, UseTree, spanned::Spanned, visit::Visit};

use super::scanned_file::ScannedFile;

type Edge = (String, String, Ref);
const MANIFEST_LIMIT: u64 = 1024 * 1024;
#[derive(Clone)]
struct Import {
    file: String,
    module: Vec<String>,
    path: Vec<String>,
    line: u32,
    raw: String,
}
struct Unit<'a> {
    sources: &'a BTreeMap<String, (String, syn::File)>,
    names: BTreeMap<Vec<String>, BTreeSet<String>>,
    modules: BTreeMap<Vec<String>, BTreeSet<String>>,
    aliases: BTreeMap<Vec<String>, BTreeSet<String>>,
    /// `use` bindings: module path plus bound name → the imports binding it.
    bindings: BTreeMap<Vec<String>, Vec<Import>>,
    /// File → the module path its top level is declared at.
    roots: BTreeMap<String, Vec<String>>,
    active: BTreeSet<String>,
    visited: BTreeSet<(String, Vec<String>)>,
    imports: Vec<Import>,
    /// Multi-segment paths in code, which only matter when rooted at a workspace crate.
    crate_paths: Vec<Import>,
    edges: Vec<Edge>,
}
impl Unit<'_> {
    fn file(&mut self, file: &str, module: Vec<String>, directory: PathBuf) {
        if module.len() > 64
            || self.active.contains(file)
            || !self.visited.insert((file.into(), module.clone()))
        {
            return;
        }
        let Some((_, syntax)) = self.sources.get(file) else {
            return;
        };
        let syntax = syntax.clone();
        self.active.insert(file.into());
        self.roots
            .entry(file.into())
            .or_insert_with(|| module.clone());
        self.modules
            .entry(module.clone())
            .or_default()
            .insert(file.into());
        let mut visitor = Scanner {
            unit: self,
            file: file.into(),
            module,
            directory,
        };
        visitor.visit_file(&syntax);
        self.active.remove(file);
    }
}
struct Scanner<'a, 'b> {
    unit: &'a mut Unit<'b>,
    file: String,
    module: Vec<String>,
    directory: PathBuf,
}
impl Scanner<'_, '_> {
    fn definition(&mut self, name: String) {
        let mut path = self.module.clone();
        path.push(name);
        self.unit
            .names
            .entry(path)
            .or_default()
            .insert(self.file.clone());
    }
    fn alias(&mut self, name: String) {
        let mut path = self.module.clone();
        path.push(name);
        self.unit
            .aliases
            .entry(path)
            .or_default()
            .insert(self.file.clone());
    }
    fn use_tree(&mut self, tree: &UseTree, prefix: Vec<String>, line: u32, raw: &str) {
        match tree {
            UseTree::Path(path) => {
                let mut next = prefix;
                next.push(path.ident.to_string());
                self.use_tree(&path.tree, next, line, raw);
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    self.use_tree(item, prefix.clone(), line, raw);
                }
            }
            _ => {
                let mut path = prefix;
                let bound = match tree {
                    UseTree::Name(name) => {
                        if name.ident != "self" {
                            path.push(name.ident.to_string());
                        }
                        path.last().cloned()
                    }
                    UseTree::Rename(rename) => {
                        if rename.ident != "self" {
                            path.push(rename.ident.to_string());
                        }
                        Some(rename.rename.to_string())
                    }
                    UseTree::Glob(_) => {
                        path.push("*".into());
                        None
                    }
                    _ => unreachable!(),
                };
                let import = Import {
                    file: self.file.clone(),
                    module: self.module.clone(),
                    path,
                    line,
                    raw: raw.into(),
                };
                if let Some(name) = bound {
                    self.alias(name.clone());
                    let mut key = self.module.clone();
                    key.push(name);
                    self.unit
                        .bindings
                        .entry(key)
                        .or_default()
                        .push(import.clone());
                }
                self.unit.imports.push(import);
            }
        }
    }
}
impl<'ast> Visit<'ast> for Scanner<'_, '_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let ident = match item {
            Item::Const(x) => Some(&x.ident),
            Item::Enum(x) => Some(&x.ident),
            Item::Fn(x) => Some(&x.sig.ident),
            Item::Static(x) => Some(&x.ident),
            Item::Struct(x) => Some(&x.ident),
            Item::Trait(x) => Some(&x.ident),
            Item::TraitAlias(x) => Some(&x.ident),
            Item::Type(x) => Some(&x.ident),
            Item::Union(x) => Some(&x.ident),
            _ => None,
        };
        if let Some(name) = ident {
            self.definition(name.to_string());
        }
        syn::visit::visit_item(self, item);
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.segments.len() > 1 {
            let line = path.span().start().line;
            let raw = self.unit.sources[&self.file]
                .0
                .lines()
                .nth(line.saturating_sub(1))
                .unwrap_or_default()
                .trim()
                .to_owned();
            self.unit.crate_paths.push(Import {
                file: self.file.clone(),
                module: self.module.clone(),
                path: path.segments.iter().map(|s| s.ident.to_string()).collect(),
                line: line as u32,
                raw,
            });
        }
        syn::visit::visit_path(self, path);
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let start = item.span().start().line;
        let end = item.span().end().line;
        let raw = self.unit.sources[&self.file]
            .0
            .lines()
            .skip(start.saturating_sub(1))
            .take(end.saturating_sub(start) + 1)
            .collect::<Vec<_>>()
            .join("\n");
        self.use_tree(&item.tree, Vec::new(), start as u32, &raw);
    }
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let name = item.ident.to_string();
        let mut module = self.module.clone();
        module.push(name.clone());
        if let Some((_, items)) = &item.content {
            self.unit
                .modules
                .entry(module.clone())
                .or_default()
                .insert(self.file.clone());
            let previous = std::mem::replace(&mut self.module, module);
            let directory = self.directory.clone();
            self.directory.push(&name);
            for item in items {
                self.visit_item(item);
            }
            self.module = previous;
            self.directory = directory;
        } else {
            let explicit = item.attrs.iter().find_map(|attr| {
                if !attr.path().is_ident("path") {
                    return None;
                }
                if let syn::Meta::NameValue(value) = &attr.meta
                    && let syn::Expr::Lit(value) = &value.value
                    && let syn::Lit::Str(path) = &value.lit
                {
                    return Some(path.value());
                }
                None
            });
            let candidates = if let Some(path) = explicit {
                // #[path] in an out-of-line module is relative to its source directory.
                vec![
                    Path::new(&self.file)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .join(path),
                ]
            } else {
                vec![
                    self.directory.join(format!("{name}.rs")),
                    self.directory.join(&name).join("mod.rs"),
                ]
            };
            let targets: Vec<_> = candidates
                .iter()
                .filter_map(|path| normalize(path))
                .filter(|path| self.unit.sources.contains_key(path))
                .collect();
            if targets.len() == 1 {
                let target = &targets[0];
                let line = item.span().start().line as u32;
                self.unit.edges.push((
                    self.file.clone(),
                    target.clone(),
                    Ref {
                        file: self.file.clone(),
                        line,
                        raw: format!("mod {name};"),
                    },
                ));
                self.unit.file(target, module, module_directory(target));
            }
        }
    }
}
fn normalize(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(s) => parts.push(s.to_str()?.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            _ => return None,
        }
    }
    Some(parts.join("/"))
}
fn module_directory(file: &str) -> PathBuf {
    let path = Path::new(file);
    let parent = path.parent().unwrap_or(Path::new(""));
    if matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("lib.rs" | "main.rs" | "mod.rs")
    ) {
        parent.into()
    } else {
        parent.join(path.file_stem().unwrap_or_default())
    }
}
/// The crate-absolute path of `import` plus the length of its scope prefix
/// (the module a `self`/`super`/bare path starts in), which is not written.
fn absolute_path(unit: &Unit<'_>, import: &Import) -> Option<(Vec<String>, usize)> {
    let mut path = import.path.clone();
    let mut base = Vec::new();
    match path.first().map(String::as_str) {
        Some("crate") => {
            path.remove(0);
        }
        Some("self") => {
            path.remove(0);
            base = import.module.clone();
        }
        Some("super") => {
            base = import.module.clone();
            while path.first().is_some_and(|p| p == "super") {
                path.remove(0);
                base.pop()?;
            }
        }
        // Since edition 2018 a bare path starts in the current module when its
        // first segment is declared or imported there; otherwise it names an
        // external crate (`std`, `serde`, …), which this index never links.
        Some(first) => {
            let mut scoped = import.module.clone();
            scoped.push(first.to_owned());
            if !(unit.names.contains_key(&scoped)
                || unit.modules.contains_key(&scoped)
                || unit.aliases.contains_key(&scoped))
            {
                return None;
            }
            base = import.module.clone();
        }
        None => {}
    }
    let scope = base.len();
    base.extend(path);
    let glob = base.last().is_some_and(|p| p == "*");
    if glob {
        base.pop();
    }
    Some((base, scope))
}
fn resolve(unit: &Unit<'_>, import: &Import) -> Option<String> {
    let (mut base, scope) = absolute_path(unit, import)?;
    let matches = unit.names.get(&base).or_else(|| unit.modules.get(&base));
    if let Some(files) = matches {
        return (files.len() == 1).then(|| files.first().unwrap().clone());
    }
    if let Some(files) = unit.aliases.get(&base)
        && files.len() == 1
        && !files.contains(&import.file)
    {
        return files.first().cloned();
    }
    // Associated items and enum variants belong to their declared type's file;
    // the walk keeps the first written segment, never the bare scope module.
    while base.len() > (scope + 1).max(1) {
        base.pop();
        if let Some(files) = unit.aliases.get(&base)
            && files.len() == 1
            && !files.contains(&import.file)
        {
            return files.first().cloned();
        }
        if let Some(files) = unit.names.get(&base) {
            return (files.len() == 1).then(|| files.first().unwrap().clone());
        }
    }
    None
}

/// Where a type path written in Rust source points, from syntax alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ItemTarget {
    /// Declared in this crate: the declaring file and the item's path inside it.
    Project { file: String, name: String },
    /// Rooted at `std`, `core` or `alloc`, as written after following `use`.
    External(String),
    /// A prelude name, an external glob, another crate (possibly a workspace
    /// member), a macro-generated item or an ambiguous binding.
    Unresolved,
}

/// Crate-local item resolution over the module tree of the import edges.
pub(crate) struct CrateItems<'a> {
    units: Vec<(String, Unit<'a>)>,
    /// File → the first unit that reaches it.
    owner: BTreeMap<String, usize>,
}
impl<'a> CrateItems<'a> {
    pub(crate) fn new(sources: &'a BTreeMap<String, (String, syn::File)>) -> Self {
        let units = units(sources);
        let mut owner = BTreeMap::new();
        for (index, (_, unit)) in units.iter().enumerate() {
            for file in unit.roots.keys() {
                owner.entry(file.clone()).or_insert(index);
            }
        }
        Self { units, owner }
    }
    /// `path` as written in `file`, inside its inline modules `inline`.
    pub(crate) fn resolve(&self, file: &str, inline: &[String], path: &[String]) -> ItemTarget {
        let Some(unit) = self.owner.get(file).map(|&index| &self.units[index].1) else {
            return ItemTarget::Unresolved;
        };
        let mut module = unit.roots[file].clone();
        module.extend_from_slice(inline);
        match item(unit, module, path, 0) {
            Some((file, path)) => {
                let root = &unit.roots[&file];
                if path.len() > root.len() && path.starts_with(root) {
                    ItemTarget::Project {
                        name: path[root.len()..].join("::"),
                        file,
                    }
                } else {
                    ItemTarget::Unresolved
                }
            }
            None if is_std(path) || std_binding(unit, path, &unit.roots[file], inline) => {
                ItemTarget::External(external_path(unit, file, inline, path))
            }
            None => ItemTarget::Unresolved,
        }
    }
}
fn is_std(path: &[String]) -> bool {
    path.len() > 1 && matches!(path[0].as_str(), "std" | "core" | "alloc")
}
/// The single `use` binding of `name` in `module`; several (cfg variants) are ambiguous.
fn binding<'u>(unit: &'u Unit<'_>, module: &[String], name: &str) -> Option<&'u Import> {
    let mut key = module.to_vec();
    key.push(name.to_owned());
    match unit.bindings.get(&key)?.as_slice() {
        [import] => Some(import),
        _ => None,
    }
}
/// Whether the first segment is bound by `use` to a `std`/`core`/`alloc` path.
fn std_binding(unit: &Unit<'_>, path: &[String], root: &[String], inline: &[String]) -> bool {
    let module = [root, inline].concat();
    path.first()
        .and_then(|first| binding(unit, &module, first))
        .is_some_and(|import| is_std(&import.path))
}
fn external_path(unit: &Unit<'_>, file: &str, inline: &[String], path: &[String]) -> String {
    let module = [unit.roots[file].as_slice(), inline].concat();
    match path.first().and_then(|first| binding(unit, &module, first)) {
        Some(import) if !is_std(path) => [&import.path[..], &path[1..]].concat().join("::"),
        _ => path.join("::"),
    }
}
/// A local item's declaring file and crate-absolute path.
type Found = Option<(String, Vec<String>)>;

/// Where `path` starts: an absolute module path and the segments after it.
/// `Err` carries the answer once a `use` binding or a glob decides.
fn start<'p>(
    unit: &Unit<'_>,
    module: Vec<String>,
    path: &'p [String],
    depth: u8,
) -> Result<(Vec<String>, &'p [String]), Found> {
    let first = path.first().ok_or(None)?;
    match first.as_str() {
        "crate" => Ok((Vec::new(), &path[1..])),
        "self" => Ok((module, &path[1..])),
        "super" => {
            let supers = path.iter().take_while(|p| *p == "super").count();
            let mut base = module;
            for _ in 0..supers {
                base.pop().ok_or(None)?;
            }
            Ok((base, &path[supers..]))
        }
        _ => {
            if let Some(import) = binding(unit, &module, first) {
                let target = [&import.path[..], &path[1..]].concat();
                return Err(item(unit, import.module.clone(), &target, depth + 1));
            }
            let scoped = [&module[..], std::slice::from_ref(first)].concat();
            if unit.names.contains_key(&scoped) || unit.modules.contains_key(&scoped) {
                Ok((module, path))
            } else {
                Err(glob(unit, &module, path, depth))
            }
        }
    }
}
/// The declaring file and crate-absolute path of a local item, following `use`
/// bindings, re-exports and globs from local modules.
fn item(unit: &Unit<'_>, module: Vec<String>, path: &[String], depth: u8) -> Found {
    if depth > 8 {
        return None;
    }
    let (mut absolute, rest) = match start(unit, module, path, depth) {
        Ok(start) => start,
        Err(found) => return found,
    };
    for (index, segment) in rest.iter().enumerate() {
        absolute.push(segment.clone());
        let last = index + 1 == rest.len();
        if (last && unit.names.contains_key(&absolute))
            || (!last && unit.modules.contains_key(&absolute))
        {
            continue;
        }
        // A `pub use` re-export inside a module.
        let (name, parent) = absolute.split_last()?;
        let import = binding(unit, parent, name)?;
        let target = [&import.path[..], &rest[index + 1..]].concat();
        return item(unit, import.module.clone(), &target, depth + 1);
    }
    match unit
        .names
        .get(&absolute)?
        .iter()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [file] => Some(((*file).clone(), absolute)),
        _ => None,
    }
}
/// A name brought in by `use local::module::*`; only a single local match counts.
fn glob(unit: &Unit<'_>, module: &[String], path: &[String], depth: u8) -> Found {
    let found: BTreeSet<_> = unit
        .imports
        .iter()
        .filter(|import| import.module == module && import.path.last().is_some_and(|p| p == "*"))
        .filter_map(|import| {
            let base = &import.path[..import.path.len() - 1];
            item(
                unit,
                import.module.clone(),
                &[base, path].concat(),
                depth + 1,
            )
        })
        .collect();
    let [only] = found.into_iter().collect::<Vec<_>>().try_into().ok()?;
    Some(only)
}

struct Manifest {
    package: String,
    /// Identifier used in code → local package it names.
    dependencies: BTreeMap<String, String>,
}
/// The package directory of a source file: the parent of its `src` ancestor.
fn crate_directory(file: &str) -> Option<&Path> {
    Path::new(file)
        .ancestors()
        .skip(1)
        .find(|dir| dir.file_name().is_some_and(|name| name == "src"))?
        .parent()
}
fn read_manifest(root: &Path, directory: &Path) -> Option<Manifest> {
    let path = directory.join("Cargo.toml");
    let bytes = crate::project_files::read_repo_file(root, &path, MANIFEST_LIMIT).ok()?;
    let table: toml::Table = std::str::from_utf8(&bytes).ok()?.parse().ok()?;
    let package = table.get("package")?.get("name")?.as_str()?.to_owned();
    // A binary reaches its own library under the package name.
    let mut dependencies = BTreeMap::from([(package.replace('-', "_"), package.clone())]);
    let declared = table.get("dependencies").and_then(|d| d.as_table());
    for (key, value) in declared.into_iter().flatten() {
        let workspace = value.get("workspace").and_then(|w| w.as_bool()) == Some(true);
        if workspace || value.get("path").is_some() {
            let target = value.get("package").and_then(|p| p.as_str()).unwrap_or(key);
            dependencies.insert(key.replace('-', "_"), target.to_owned());
        }
    }
    Some(Manifest {
        package,
        dependencies,
    })
}
/// Hash the manifests that decide crate edges for these Rust sources.
pub(crate) fn hash_manifests<'a>(
    root: &Path,
    rust_files: impl Iterator<Item = &'a str>,
    hash: &mut Sha256,
) {
    let directories: BTreeSet<_> = rust_files.filter_map(crate_directory).collect();
    for directory in directories {
        let path = directory.join("Cargo.toml");
        if let Ok(bytes) = crate::project_files::read_repo_file(root, &path, MANIFEST_LIMIT) {
            hash.update(path.to_string_lossy().as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
}
/// Edges from imports and paths rooted at a declared workspace crate. A package
/// name found in two directories is ambiguous and links nowhere.
fn workspace_edges(root: &Path, units: &[(String, Unit<'_>)]) -> Vec<Edge> {
    let mut manifests = BTreeMap::new();
    let mut packages: BTreeMap<String, Vec<&Path>> = BTreeMap::new();
    for directory in units
        .iter()
        .filter_map(|(file, _)| crate_directory(file))
        .collect::<BTreeSet<_>>()
    {
        if let Some(manifest) = read_manifest(root, directory) {
            packages
                .entry(manifest.package.clone())
                .or_default()
                .push(directory);
            manifests.insert(directory, manifest);
        }
    }
    let libraries: BTreeMap<_, _> = units.iter().map(|(file, unit)| (file, unit)).collect();
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    for (file, unit) in units {
        let Some(manifest) = crate_directory(file).and_then(|d| manifests.get(d)) else {
            continue;
        };
        for import in unit.imports.iter().chain(&unit.crate_paths) {
            let Some(first) = import.path.first() else {
                continue;
            };
            let Some(package) = manifest.dependencies.get(first) else {
                continue;
            };
            let [directory] = packages.get(package).map_or(&[][..], Vec::as_slice) else {
                continue;
            };
            let library = directory.join("src/lib.rs").to_string_lossy().into_owned();
            // A local module of that name shadows the crate.
            if unit.modules.contains_key(std::slice::from_ref(first)) {
                continue;
            }
            let Some(target_unit) = libraries.get(&library) else {
                continue;
            };
            let mut inner = import.clone();
            inner.path[0] = "crate".into();
            inner.module = Vec::new();
            let target = resolve(target_unit, &inner).unwrap_or(library);
            if target != import.file
                && seen.insert((import.file.clone(), target.clone(), import.line))
            {
                edges.push((
                    import.file.clone(),
                    target,
                    Ref {
                        file: import.file.clone(),
                        line: import.line,
                        raw: import.raw.clone(),
                    },
                ));
            }
        }
    }
    edges
}

/// Names that globs of known local modules bind, never external wildcards.
fn glob_additions(unit: &Unit<'_>) -> Vec<(Vec<String>, String)> {
    let mut additions = Vec::new();
    for import in &unit.imports {
        if !import.path.last().is_some_and(|p| p == "*") {
            continue;
        }
        let Some((module, _)) = absolute_path(unit, import) else {
            continue;
        };
        if !unit.modules.contains_key(&module) {
            continue;
        }
        for name in unit
            .names
            .keys()
            .chain(unit.aliases.keys())
            .chain(unit.modules.keys())
        {
            if name.len() == module.len() + 1 && name.starts_with(&module) {
                let mut alias = import.module.clone();
                alias.push(name.last().unwrap().clone());
                additions.push((alias, import.file.clone()));
            }
        }
    }
    additions
}
/// Bind glob imports until nothing changes. The finite symbol inventory
/// bounds propagation even for cyclic re-exports.
fn propagate_globs(unit: &mut Unit<'_>) {
    for _ in 0..64 {
        let mut changed = false;
        for (alias, file) in glob_additions(unit) {
            changed |= unit.aliases.entry(alias).or_default().insert(file);
        }
        if !changed {
            break;
        }
    }
}

/// One unit per crate root: `lib.rs`/`main.rs` first, then every unattached
/// source file as its own root rather than guessing another crate's scope.
fn units(sources: &BTreeMap<String, (String, syn::File)>) -> Vec<(String, Unit<'_>)> {
    let mut roots: Vec<_> = sources
        .keys()
        .filter(|p| {
            matches!(
                Path::new(p).file_name().and_then(|p| p.to_str()),
                Some("lib.rs" | "main.rs")
            )
        })
        .cloned()
        .collect();
    roots.extend(
        sources
            .keys()
            .filter(|p| !roots.contains(p))
            .cloned()
            .collect::<Vec<_>>(),
    );
    let mut visited = BTreeSet::new();
    let mut units = Vec::new();
    for file in roots {
        if visited.contains(&file) {
            continue;
        }
        let mut unit = Unit {
            sources,
            names: BTreeMap::new(),
            modules: BTreeMap::new(),
            aliases: BTreeMap::new(),
            bindings: BTreeMap::new(),
            roots: BTreeMap::new(),
            active: BTreeSet::new(),
            visited: BTreeSet::new(),
            imports: Vec::new(),
            crate_paths: Vec::new(),
            edges: Vec::new(),
        };
        unit.file(&file, Vec::new(), module_directory(&file));
        propagate_globs(&mut unit);
        visited.extend(unit.visited.iter().map(|(file, _)| file.clone()));
        units.push((file, unit));
    }
    units
}

/// Edges of one unit from its `use` imports and qualified paths in code.
fn import_edges(
    unit: &Unit<'_>,
    names: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Vec<Edge> {
    let edge = |from: &str, to: String, line: u32, raw: &str| {
        let evidence = Ref {
            file: from.to_owned(),
            line,
            raw: raw.to_owned(),
        };
        (from.to_owned(), to, evidence)
    };
    let mut edges = Vec::new();
    for import in &unit.imports {
        if let Some(target) = resolve(unit, import) {
            if target != import.file {
                edges.push(edge(&import.file, target, import.line, &import.raw));
            }
        } else if matches!(
            import.path.first().map(String::as_str),
            Some("crate" | "super" | "self")
        ) {
            warnings.push(format!(
                "unresolved Rust-local import `{}` in {} (line {}): {}",
                import.path.join("::"),
                names[&import.file],
                import.line,
                import.raw
            ));
        }
    }
    // Qualified paths in code (`crate::b::Repo::new()`) link without a `use`;
    // unresolved ones are mostly external (`Vec::new`) and stay silent.
    let mut seen = BTreeSet::new();
    for path in &unit.crate_paths {
        if let Some(target) = resolve(unit, path)
            && target != path.file
            && seen.insert((path.file.clone(), target.clone(), path.line))
        {
            edges.push(edge(&path.file, target, path.line, &path.raw));
        }
    }
    edges
}

pub(super) fn edges(
    root: &Path,
    files: &[ScannedFile],
) -> Result<(Vec<Edge>, Vec<String>), ArchguardError> {
    let mut sources = BTreeMap::new();
    let mut warnings = Vec::new();
    for file in files.iter().filter(|f| f.lang == "rs") {
        let source =
            String::from_utf8_lossy(&std::fs::read(root.join(&file.rel_path))?).into_owned();
        match syn::parse_file(&source) {
            Ok(syntax) => {
                sources.insert(file.rel_path.clone(), (source, syntax));
            }
            Err(error) => warnings.push(format!(
                "Rust import analysis could not parse {}: {error}",
                file.stripped
            )),
        }
    }
    let names: BTreeMap<_, _> = files
        .iter()
        .map(|f| (f.rel_path.clone(), f.stripped.clone()))
        .collect();
    let mut units = units(&sources);
    let mut edges = Vec::new();
    for (_, unit) in &mut units {
        edges.append(&mut unit.edges);
        edges.extend(import_edges(unit, &names, &mut warnings));
    }
    edges.extend(workspace_edges(root, &units));
    let edges = edges
        .into_iter()
        .filter(|(from, to, _)| from != to)
        .map(|(from, to, mut evidence)| {
            evidence.file = names[&from].clone();
            (names[&from].clone(), names[&to].clone(), evidence)
        })
        .collect();
    Ok((edges, warnings))
}
