//! Parsed Terraform modules and the walk over their instances (ADR 0015).
use crate::hcl::{attr, literal};
use archguard_core::{ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{Discovery, diagnostic, relative};
use hcl_edit::structure::Block;
use hcl_edit::structure::Body;
use hcl_edit::{Span, parser};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Component, Path};
use std::rc::Rc;

const MAX_FILES: usize = 2000;
const MAX_ENTRIES: usize = 50_000;
const MAX_MODULE_DEPTH: usize = 8;

/// One parsed `.tf` file.
pub struct TfFile {
    pub path: String,
    /// Byte offset of each line start.
    line_at: Vec<usize>,
    body: Body,
}

impl TfFile {
    pub fn line(&self, span: Option<Range<usize>>) -> Option<u32> {
        let start = span?.start;
        u32::try_from(self.line_at.partition_point(|&s| s <= start)).ok()
    }
}

/// A `resource` block within one module instance.
pub struct Resource<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    /// Full address, e.g. `module.db.aws_db_instance.main`.
    pub address: String,
    pub block: &'a Block,
    pub file: &'a TfFile,
    pub module: Rc<ModuleCtx<'a>>,
}

/// One module instance: its `variable` blocks, the `module` block that called it and the
/// caller's context. Lets readers resolve `var.x` without a second parse.
pub struct ModuleCtx<'a> {
    variables: Vec<(&'a Block, &'a TfFile)>,
    /// `locals` blocks and `resource` blocks of this instance only; read by ES detail (ADR 0023).
    pub locals: Vec<&'a Block>,
    pub resources: Vec<&'a Block>,
    /// `terraform` blocks of this instance (`required_providers`); read by the ES provider sources.
    pub terraform: Vec<&'a Block>,
    call: Option<(&'a Block, &'a TfFile)>,
    parent: Option<Rc<ModuleCtx<'a>>>,
    /// Module nesting level; the root is 0.
    depth: usize,
}

impl<'a> ModuleCtx<'a> {
    pub fn variable(&self, name: &str) -> Option<(&'a Block, &'a TfFile)> {
        let named = |b: &&(&Block, &TfFile)| b.0.labels.first().is_some_and(|l| l.as_str() == name);
        self.variables.iter().find(named).copied()
    }
    pub fn call(&self) -> Option<(&'a Block, &'a TfFile)> {
        self.call
    }
    pub fn parent(&self) -> Option<&Rc<ModuleCtx<'a>>> {
        self.parent.as_ref()
    }
}

impl Resource<'_> {
    pub fn line(&self) -> Option<u32> {
        self.file.line(self.block.span())
    }
    pub fn evidence(&self, line: Option<u32>, suffix: &str, note: &str) -> ServiceEvidence {
        let file = &self.file.path;
        let at = line.map_or(String::new(), |l| format!(":{l}"));
        ServiceEvidence {
            file: file.clone(),
            pointer: format!("/{}{suffix}", self.address.replace('.', "/")),
            line,
            description: format!(
                "Declared in Terraform resource {} ({file}{at}). {note}",
                self.address
            ),
        }
    }
}

/// Calls `each_instance` once per module instance of every root module under `root`.
/// `read` reads one file below `root` (the caller decides how safely).
pub fn walk(
    root: &Path,
    discovery: &Discovery,
    report: &mut ServiceTopology,
    read: &dyn Fn(&Path, &Path) -> Option<String>,
    each_instance: &mut dyn FnMut(&mut ServiceTopology, &str, &[Resource]),
) {
    let modules = parse(root, read, discovery, report);
    let called = module_calls(&modules, report);
    for dir in modules.keys().filter(|d| !called.contains(*d)) {
        let scope = format!("terraform:{}", if dir.is_empty() { "." } else { dir });
        let mut instances = Vec::new();
        let mut deep = BTreeSet::new();
        instance(&modules, dir, "", None, &mut instances, &mut deep);
        for file in deep {
            let message = format!(
                "Terraform modules are nested deeper than {MAX_MODULE_DEPTH} levels; the modules called below are not analyzed and their resources are unknown."
            );
            diagnostic(report, &file, None, message);
        }
        for resources in &instances {
            each_instance(report, &scope, resources);
        }
    }
}

/// Parsed `.tf` files by module directory (`""` for the analyzed root).
fn parse(
    root: &Path,
    read: &dyn Fn(&Path, &Path) -> Option<String>,
    discovery: &Discovery,
    report: &mut ServiceTopology,
) -> BTreeMap<String, Vec<TfFile>> {
    let mut modules: BTreeMap<String, Vec<TfFile>> = BTreeMap::new();
    let mut count = 0;
    for (index, entry) in discovery.walk(root).enumerate() {
        let Ok(entry) = entry else { continue };
        let tf = entry.file_type().is_some_and(|t| t.is_file())
            && entry.path().extension().is_some_and(|x| x == "tf")
            // Remote modules downloaded by `terraform init` are not analyzed.
            && !entry.path().components().any(|c| c.as_os_str() == ".terraform");
        count += usize::from(tf);
        if index >= MAX_ENTRIES || count > MAX_FILES {
            let none = if count == 0 {
                " (no .tf file found before the limit)"
            } else {
                ""
            };
            let message = format!(
                "Terraform scan stopped after {MAX_ENTRIES} entries or {MAX_FILES} .tf files{none}; later .tf files are not read and their resources are unknown."
            );
            diagnostic(report, ".", None, message);
            break;
        }
        if !tf {
            continue;
        }
        let path = relative(root, entry.path());
        let parsed = read(root, entry.path()).map(|text| (parser::parse_body(&text), text));
        let Some((Ok(body), text)) = parsed else {
            diagnostic(
                report,
                &path,
                None,
                "Terraform file could not be read or is not valid HCL; its resources are unknown.",
            );
            continue;
        };
        let dir = Path::new(&path)
            .parent()
            .and_then(Path::to_str)
            .unwrap_or("")
            .to_owned();
        let line_at: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        modules.entry(dir).or_default().push(TfFile {
            path,
            line_at,
            body,
        });
    }
    modules
}

fn blocks<'a>(file: &'a TfFile, ident: &'a str) -> impl Iterator<Item = &'a Block> {
    file.body
        .blocks()
        .filter(move |b| b.ident.as_str() == ident)
}

/// The directory of a local module source (`./…`, `../…`) inside the analyzed root.
fn local_source(dir: &str, source: &str) -> Option<String> {
    if !(source.starts_with("./") || source.starts_with("../")) {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for c in Path::new(source).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(p) => parts.push(p.to_str()?),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// Directories called as local modules; remote or unreadable modules are reported as gaps.
fn module_calls(
    modules: &BTreeMap<String, Vec<TfFile>>,
    report: &mut ServiceTopology,
) -> BTreeSet<String> {
    let mut called = BTreeSet::new();
    let calls = modules.iter().flat_map(|(dir, files)| {
        files
            .iter()
            .flat_map(move |file| blocks(file, "module").map(move |block| (dir, file, block)))
    });
    for (dir, file, block) in calls {
        match module_call(modules, dir, block) {
            Ok(child) => {
                called.insert(child);
            }
            Err(message) => diagnostic(report, &file.path, None, message),
        }
    }
    called
}

/// The directory of a local module call, or why its resources are unknown.
fn module_call(
    modules: &BTreeMap<String, Vec<TfFile>>,
    dir: &str,
    block: &Block,
) -> Result<String, String> {
    let Some(s) = attr(&block.body, "source").and_then(literal) else {
        return Err("Terraform module source is not a literal; its resources are unknown.".into());
    };
    let local = s.starts_with("./") || s.starts_with("../");
    match local_source(dir, &s) {
        Some(child) if modules.contains_key(&child) => Ok(child),
        Some(_) => Err(format!(
            "Local Terraform module {s} has no readable .tf files inside the analyzed directory; its resources are unknown."
        )),
        None if local => Err(format!(
            "Local Terraform module {s} leaves the analyzed directory; its resources are unknown."
        )),
        None => Err(format!(
            "Remote Terraform module {s} is not fetched; its resources are unknown."
        )),
    }
}

/// The resources of the module in `dir` and, recursively, of the local modules it calls;
/// `deep` collects the files whose module calls lie beyond the nesting limit.
fn instance<'a>(
    modules: &'a BTreeMap<String, Vec<TfFile>>,
    dir: &str,
    prefix: &str,
    caller: Option<(Rc<ModuleCtx<'a>>, &'a Block, &'a TfFile)>,
    out: &mut Vec<Vec<Resource<'a>>>,
    deep: &mut BTreeSet<String>,
) {
    let Some(files) = modules.get(dir) else {
        return;
    };
    let ctx = Rc::new(ModuleCtx {
        variables: files
            .iter()
            .flat_map(|f| blocks(f, "variable").map(move |b| (b, f)))
            .collect(),
        locals: files.iter().flat_map(|f| blocks(f, "locals")).collect(),
        resources: files.iter().flat_map(|f| blocks(f, "resource")).collect(),
        terraform: files.iter().flat_map(|f| blocks(f, "terraform")).collect(),
        call: caller.as_ref().map(|c| (c.1, c.2)),
        depth: caller.as_ref().map_or(0, |c| c.0.depth + 1),
        parent: caller.map(|c| c.0),
    });
    let depth = ctx.depth;
    let mut resources = Vec::new();
    for file in files {
        for block in blocks(file, "resource") {
            if let [kind, name] = block.labels.as_slice() {
                let (kind, name) = (kind.as_str(), name.as_str());
                resources.push(Resource {
                    kind,
                    name,
                    address: format!("{prefix}{kind}.{name}"),
                    block,
                    file,
                    module: Rc::clone(&ctx),
                });
            }
        }
        for block in blocks(file, "module") {
            if depth >= MAX_MODULE_DEPTH {
                deep.insert(file.path.clone());
                continue;
            }
            let child = attr(&block.body, "source")
                .and_then(literal)
                .and_then(|s| local_source(dir, &s));
            if let (Some(child), [name]) = (child, block.labels.as_slice()) {
                let prefix = format!("{prefix}module.{}.", name.as_str());
                let caller = Some((Rc::clone(&ctx), block, file));
                instance(modules, &child, &prefix, caller, out, deep);
            }
        }
    }
    out.push(resources);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_module_sources_stay_inside_the_analyzed_directory() {
        assert_eq!(
            local_source("infra", "./modules/db").as_deref(),
            Some("infra/modules/db")
        );
        assert_eq!(
            local_source("infra/prod", "../shared").as_deref(),
            Some("infra/shared")
        );
        assert_eq!(local_source("", "../outside"), None);
        assert_eq!(local_source("", "git::https://example.org/m.git"), None);
    }

    #[test]
    fn the_entry_limit_without_any_tf_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..MAX_ENTRIES {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "").unwrap();
        }
        let mut report = ServiceTopology::default();
        let discovery = Discovery::new(dir.path(), &mut report).unwrap();
        assert!(parse(dir.path(), &|_, _| None, &discovery, &mut report).is_empty());
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("no .tf file found before the limit")),
            "{:?}",
            report.diagnostics
        );
    }
}
