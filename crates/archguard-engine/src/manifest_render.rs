//! Opt-in rendering of Helm charts and Kustomize overlays with the official tools.
//!
//! Rendering starts external programs, so it runs only after the operator sets
//! [`OPT_IN`]`=1`. The tools get an empty environment with temporary home, cache
//! and config directories, no kubeconfig and no `PATH`, so they find no plugins,
//! repositories or credentials. ArchGuard never runs `helm dependency
//! update/build`, `--dependency-update`, `--validate` or `--enable-dns`, and never
//! passes kustomize `--enable-helm`, `--enable-exec` or `--enable-alpha-plugins`.
//! A kustomization that names a remote, missing or out-of-root location is not run,
//! because kustomize would try to fetch it. The rendered YAML goes through the
//! plain-manifest reader, which drops Secrets unread.
use super::support::{diagnostic, relative};
use archguard_core::ServiceTopology;
use serde_yaml_ng::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Set to `1` to render Helm charts and Kustomize overlays.
pub(crate) const OPT_IN: &str = "ARCHGUARD_RENDER_MANIFESTS";
const KUSTOMIZATIONS: [&str; 3] = ["kustomization.yaml", "kustomization.yml", "Kustomization"];
const CHART: &str = "Chart.yaml";
const NAMESPACE: &str = "default";
/// Documents taken from one rendered chart or overlay, as for one plain manifest file.
const MAX_DOCUMENTS: usize = 2000;
/// Files and directories checked for symlinks in one chart.
const MAX_CHART_ENTRIES: usize = 10_000;

/// The rendering tools the operator allows; `None` when not found.
pub(crate) struct Tools {
    pub enabled: bool,
    pub helm: Option<PathBuf>,
    /// The kustomize executable and the subcommand before the overlay path.
    pub kustomize: Option<(PathBuf, &'static str)>,
}
impl Tools {
    pub(crate) fn from_env() -> Self {
        let enabled = std::env::var_os(OPT_IN).is_some_and(|value| value == "1");
        if !enabled {
            return Self {
                enabled,
                helm: None,
                kustomize: None,
            };
        }
        let kustomize = match std::env::var_os("ARCHGUARD_KUSTOMIZE") {
            Some(name) => executable(&name).map(|path| {
                let kubectl = path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().starts_with("kubectl"));
                let command = if kubectl { "kustomize" } else { "build" };
                (path, command)
            }),
            None => executable("kustomize".as_ref())
                .map(|path| (path, "build"))
                .or_else(|| executable("kubectl".as_ref()).map(|path| (path, "kustomize"))),
        };
        Self {
            enabled,
            helm: executable(&std::env::var_os("ARCHGUARD_HELM").unwrap_or_else(|| "helm".into())),
            kustomize,
        }
    }
}
/// A path names the executable itself; a bare name is looked up on `PATH`.
pub(crate) fn executable(name: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }
    let file = format!("{}{}", name.to_string_lossy(), std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Part of the tree that a successful render replaces in the plain-manifest reader.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Region {
    /// A directory with all subdirectories (a Helm chart).
    Tree(String),
    /// The files directly in a directory (a kustomization directory).
    Dir(String),
    File(String),
}
impl Region {
    fn covers(&self, file: &str) -> bool {
        match self {
            Region::Tree(dir) => dir.is_empty() || file.starts_with(&format!("{dir}/")),
            Region::Dir(dir) => parent(file) == dir,
            Region::File(path) => path == file,
        }
    }
}
/// Documents of one rendered chart or overlay.
pub(crate) struct Render {
    /// The chart or overlay directory, the deployment scope of the documents.
    pub bundle: String,
    /// `(evidence file, document)`; Secrets are still included and dropped by the reader.
    pub documents: Vec<(String, Value)>,
    pub regions: BTreeSet<Region>,
    /// Appended to every evidence description that points into this render.
    pub note: String,
}
impl Render {
    pub(crate) fn covers(&self, file: &str) -> bool {
        self.regions.iter().any(|region| region.covers(file))
    }
}

/// A root-relative file, read without following symlinks.
fn read(root: &Path, file: &str) -> Option<String> {
    crate::project_files::read_manifest(root, Path::new(file)).ok()
}
fn parent(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(dir, _)| dir)
}
fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}
/// Whether `file` (root-relative, `/`-separated) is a Chart.yaml or kustomization.
pub(super) fn is_source(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    name == CHART || KUSTOMIZATIONS.contains(&name)
}

/// Renders the top-level charts and overlays among `sources`, or explains why not.
pub(super) fn render(
    root: &Path,
    report: &mut ServiceTopology,
    sources: &[String],
    tools: &Tools,
) -> Vec<Render> {
    let charts: Vec<&str> = sources
        .iter()
        .filter(|file| file.rsplit('/').next() == Some(CHART))
        .map(String::as_str)
        .collect();
    // A chart inside another chart's directory is a subchart rendered with its parent.
    let charts: Vec<&str> = charts
        .iter()
        .copied()
        .filter(|chart| {
            !charts
                .iter()
                .any(|other| other != chart && Region::Tree(parent(other).into()).covers(chart))
        })
        .collect();
    let overlays = top_level_kustomizations(root, sources);
    if !tools.enabled {
        for chart in &charts {
            diagnostic(
                report,
                chart,
                None,
                format!(
                    "Helm chart is not rendered, so its services and connections stay unknown; templates are not executed. Set {OPT_IN}=1 to render it offline with helm template (helm on PATH or ARCHGUARD_HELM)."
                ),
            );
        }
        for overlay in &overlays {
            diagnostic(
                report,
                overlay,
                None,
                format!(
                    "Kustomize overlay is not applied; its files are read only as plain manifests. Set {OPT_IN}=1 to render it offline with kustomize build (kustomize or kubectl on PATH, or ARCHGUARD_KUSTOMIZE)."
                ),
            );
        }
        return Vec::new();
    }
    let mut renders: Vec<Render> = Vec::new();
    for chart in charts {
        match helm(root, chart, tools.helm.as_deref()) {
            Ok(render) => {
                diagnostic(
                    report,
                    chart,
                    None,
                    "Rendered offline with helm template using only the chart's values.yaml; values from other values files, --set, lookup (empty without a cluster) and the cluster stay unknown.",
                );
                renders.push(render);
            }
            Err(message) => diagnostic(
                report,
                chart,
                None,
                format!(
                    "Helm chart is not rendered; its templates are read only as plain YAML. {message}"
                ),
            ),
        }
    }
    for overlay in overlays {
        if renders.iter().any(|render| render.covers(&overlay)) {
            continue;
        }
        match kustomize(root, &overlay, tools.kustomize.as_ref()) {
            Ok(render) => renders.push(render),
            Err(message) => diagnostic(
                report,
                &overlay,
                None,
                format!(
                    "Kustomize overlay is not applied; its files are read only as plain manifests. {message}"
                ),
            ),
        }
    }
    renders
}

/// Kustomizations no other kustomization consumes, e.g. overlays but not their bases.
pub(crate) fn top_level_kustomizations(root: &Path, sources: &[String]) -> Vec<String> {
    let kustomizations: Vec<(&String, BTreeSet<Region>)> = sources
        .iter()
        .filter(|file| KUSTOMIZATIONS.contains(&file.rsplit('/').next().unwrap_or("")))
        .map(|file| {
            let mut regions = BTreeSet::new();
            let _ = inspect(root, file, &mut BTreeSet::new(), &mut regions);
            (file, regions)
        })
        .collect();
    kustomizations
        .iter()
        .filter(|(file, _)| {
            !kustomizations.iter().any(|(other, regions)| {
                other != file && regions.iter().any(|region| region.covers(file))
            })
        })
        .map(|(file, _)| (*file).clone())
        .collect()
}

/// Collects the local paths a kustomization reads, following directory resources.
/// Fails when it names anything kustomize would fetch or that lies outside `root`.
pub(crate) fn inspect(
    root: &Path,
    file: &str,
    seen: &mut BTreeSet<String>,
    regions: &mut BTreeSet<Region>,
) -> Result<(), String> {
    if !seen.insert(file.to_owned()) {
        return Ok(());
    }
    if seen.len() > 200 {
        return Err("More than 200 nested kustomizations.".into());
    }
    let text = read(root, file)
        .ok_or_else(|| format!("{file} could not be read within the safe file limits."))?;
    let doc: Value =
        serde_yaml_ng::from_str(&text).map_err(|_| format!("{file} is not valid YAML."))?;
    let dir = parent(file);
    regions.insert(Region::Dir(dir.into()));
    for key in ["helmCharts", "helmChartInflationGenerator", "helmGlobals"] {
        if !doc[key].is_null() {
            return Err(format!(
                "{file} declares {key}, which needs kustomize --enable-helm; ArchGuard never enables it."
            ));
        }
    }
    for (field, entry) in references(&doc) {
        let Some(path) = local(root, dir, entry) else {
            return Err(format!(
                "{file}: an entry of {field} names a remote, missing or out-of-root location, which kustomize would try to fetch; it is not run."
            ));
        };
        if !root.join(&path).is_dir() {
            regions.insert(Region::File(path));
            continue;
        }
        regions.insert(Region::Dir(path.clone()));
        // Every field kustomize resolves as a directory builds its kustomization.
        if let Some(name) = KUSTOMIZATIONS
            .iter()
            .find(|name| root.join(&path).join(name).is_file())
        {
            inspect(root, &join(&path, name), seen, regions)?;
        }
    }
    Ok(())
}
/// The path entries of a kustomization, by field.
fn references(doc: &Value) -> Vec<(&'static str, &str)> {
    let mut out = Vec::new();
    for field in [
        "resources",
        "bases",
        "components",
        "crds",
        "configurations",
        "generators",
        "transformers",
        "validators",
        "patchesStrategicMerge",
    ] {
        for entry in doc[field].as_sequence().into_iter().flatten() {
            // Inline patches and plugin configs are YAML, not paths.
            if let Some(path) = entry.as_str().filter(|s| !s.contains('\n')) {
                out.push((field, path));
            }
        }
    }
    for field in ["patches", "patchesJson6902", "replacements"] {
        for entry in doc[field].as_sequence().into_iter().flatten() {
            if let Some(path) = entry["path"].as_str() {
                out.push((field, path));
            }
        }
    }
    generator_files(doc, &mut out);
    if let Some(path) = doc["openapi"]["path"].as_str() {
        out.push(("openapi", path));
    }
    out
}
/// Files read by ConfigMap and Secret generators (`files` entries may be `key=path`).
fn generator_files<'v>(doc: &'v Value, out: &mut Vec<(&'static str, &'v str)>) {
    for field in ["configMapGenerator", "secretGenerator"] {
        for generator in doc[field].as_sequence().into_iter().flatten() {
            let files = generator["files"].as_sequence().into_iter().flatten();
            let envs = generator["envs"].as_sequence().into_iter().flatten();
            for entry in files.chain(envs).filter_map(Value::as_str) {
                out.push((field, entry.split_once('=').map_or(entry, |(_, path)| path)));
            }
            if let Some(env) = generator["env"].as_str() {
                out.push((field, env));
            }
        }
    }
}
/// The root-relative path of an existing entry inside `root`, after symlinks.
pub(crate) fn local(root: &Path, dir: &str, entry: &str) -> Option<String> {
    if entry.is_empty() || entry.contains("://") || entry.starts_with("git@") {
        return None;
    }
    let path = std::fs::canonicalize(root.join(dir).join(entry)).ok()?;
    path.starts_with(root).then(|| relative(root, &path))
}

/// A chart path helm could read outside `root`: a symlink pointing out of it, or
/// any path that could not be checked within the entry limit. Symlinked
/// directories inside `root` are walked too, since helm follows them.
fn escaping_symlink(root: &Path, dir: &Path) -> Option<String> {
    let mut stack = vec![dir.to_path_buf()];
    let mut visited: BTreeSet<PathBuf> = std::fs::canonicalize(dir).into_iter().collect();
    let mut entries = 0;
    while let Some(next) = stack.pop() {
        let Ok(listing) = std::fs::read_dir(&next) else {
            return Some(relative(root, &next));
        };
        for entry in listing {
            entries += 1;
            let Some((path, kind)) = entry
                .ok()
                .and_then(|e| Some((e.path(), e.file_type().ok()?)))
            else {
                return Some(relative(root, &next));
            };
            if entries > MAX_CHART_ENTRIES {
                return Some(relative(root, dir));
            }
            match descend(root, path, kind, &mut visited) {
                Ok(Some(dir)) => stack.push(dir),
                Ok(None) => {}
                Err(escape) => return Some(escape),
            }
        }
    }
    None
}

/// Whether the walk enters `path`: a directory, or a symlink to a directory
/// inside `root` not seen before. A symlink out of `root` is the escape.
fn descend(
    root: &Path,
    path: PathBuf,
    kind: std::fs::FileType,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    if !kind.is_symlink() {
        return Ok(kind.is_dir().then_some(path));
    }
    let Some(target) = std::fs::canonicalize(&path)
        .ok()
        .filter(|p| p.starts_with(root))
    else {
        return Err(relative(root, &path));
    };
    Ok((target.is_dir() && visited.insert(target)).then_some(path))
}
/// A command with an empty environment and temporary home, cache and config dirs.
fn isolated(executable: &Path, home: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("KUBECONFIG", home.join("no-kubeconfig"));
    if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
        command.env("SYSTEMROOT", system_root);
    }
    command
}
/// Runs the command within the shared time and output budget; errors keep one line.
fn run(command: &mut Command, label: &str, hint: &str) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let bytes = crate::go_calls::execute(command, dir.path(), None, label, hint).map_err(|e| {
        let line: String = e.lines().next().unwrap_or("").chars().take(300).collect();
        let budget = if line.contains("budget exceeded") {
            " Nothing from it is used; coverage is partial."
        } else {
            ""
        };
        format!("{line}{budget}")
    })?;
    String::from_utf8(bytes).map_err(|_| format!("{label} output is not UTF-8."))
}

const HELM_HINT: &str = "Install helm or set ARCHGUARD_HELM to its executable.";
fn helm(root: &Path, chart: &str, helm: Option<&Path>) -> Result<Render, String> {
    let helm = helm.ok_or(format!("helm was not found. {HELM_HINT}"))?;
    let dir = parent(chart);
    let name = read(root, chart)
        .and_then(|text| serde_yaml_ng::from_str::<Value>(&text).ok())
        .and_then(|doc| doc["name"].as_str().map(str::to_owned))
        .ok_or("Chart.yaml has no readable name.")?;
    if let Some(link) = escaping_symlink(root, &root.join(dir)) {
        return Err(format!(
            "{link} could not be checked or is a symlink outside the analyzed root (limit {MAX_CHART_ENTRIES} entries per chart); helm would read it."
        ));
    }
    let home = tempfile::tempdir().map_err(|e| e.to_string())?;
    let h = home.path();
    let mut command = isolated(helm, h);
    command
        .env("HELM_CACHE_HOME", h.join("cache"))
        .env("HELM_CONFIG_HOME", h.join("config"))
        .env("HELM_DATA_HOME", h.join("data"))
        .env("HELM_PLUGINS", h.join("plugins"))
        .env("HELM_REGISTRY_CONFIG", h.join("registry.json"))
        .env("HELM_REPOSITORY_CONFIG", h.join("repositories.yaml"))
        .env("HELM_REPOSITORY_CACHE", h.join("repository"))
        .arg("template")
        .arg(&name)
        .arg(root.join(dir))
        .args(["--namespace", NAMESPACE, "--skip-tests"]);
    let text = run(&mut command, "helm template", HELM_HINT).map_err(|message| {
        if message.contains("missing in charts/") {
            format!("{message} ArchGuard never runs helm dependency build or --dependency-update, which need the network; vendor the dependencies into charts/ to render this chart.")
        } else {
            message
        }
    })?;
    let display = if dir.is_empty() { "." } else { dir };
    Ok(Render {
        bundle: dir.into(),
        documents: documents(&text, Some(dir), chart)?,
        regions: BTreeSet::from([Region::Tree(dir.into())]),
        note: format!(
            "Rendered by helm template from {display} (release {name}, namespace {NAMESPACE}, only the chart's values.yaml); not a literal manifest. Values from other values files, --set and lookup stay unknown."
        ),
    })
}

const KUSTOMIZE_HINT: &str =
    "Install kustomize or kubectl, or set ARCHGUARD_KUSTOMIZE to its executable.";
fn kustomize(
    root: &Path,
    overlay: &str,
    tool: Option<&(PathBuf, &'static str)>,
) -> Result<Render, String> {
    let mut regions = BTreeSet::new();
    inspect(root, overlay, &mut BTreeSet::new(), &mut regions)?;
    let (executable, subcommand) =
        tool.ok_or(format!("kustomize was not found. {KUSTOMIZE_HINT}"))?;
    let dir = parent(overlay);
    let home = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut command = isolated(executable, home.path());
    command.arg(subcommand).arg(root.join(dir));
    let tool_name = if *subcommand == "kustomize" {
        "kubectl kustomize"
    } else {
        "kustomize build"
    };
    let text = run(&mut command, tool_name, KUSTOMIZE_HINT)?;
    let display = if dir.is_empty() { "." } else { dir };
    Ok(Render {
        bundle: dir.into(),
        documents: documents(&text, None, overlay)?,
        regions,
        note: format!("Rendered by {tool_name} from {display}; not a literal manifest."),
    })
}

/// Splits rendered YAML into documents. Helm's `# Source:` comments point each
/// document to its template file inside `chart`; otherwise it points to `fallback`.
pub(crate) fn documents(
    text: &str,
    chart: Option<&str>,
    fallback: &str,
) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    let mut chunk = String::new();
    for line in text.lines().chain(std::iter::once("---")) {
        if line.trim_end() != "---" {
            chunk.push_str(line);
            chunk.push('\n');
            continue;
        }
        let doc: Value = serde_yaml_ng::from_str(&chunk)
            .map_err(|_| "Rendered output is not valid YAML; nothing from it is used.")?;
        let source = chart.and_then(|dir| template(dir, &chunk));
        chunk.clear();
        if doc.is_null() {
            continue;
        }
        if out.len() >= MAX_DOCUMENTS {
            return Err(format!(
                "Rendered output has more than {MAX_DOCUMENTS} documents; nothing from it is used and coverage is partial."
            ));
        }
        out.push((source.unwrap_or_else(|| fallback.to_owned()), doc));
    }
    Ok(out)
}
/// The template file named by Helm's `# Source: <chart>/<path>` comment.
fn template(dir: &str, chunk: &str) -> Option<String> {
    let source = chunk
        .lines()
        .find_map(|line| line.strip_prefix("# Source: "))?
        .trim();
    let (_, path) = source.split_once('/')?;
    Path::new(path)
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
        .then(|| join(dir, path))
}

/// Marks evidence from rendered documents so it can be told from literal manifests.
pub(super) fn mark(report: &mut ServiceTopology, notes: &BTreeMap<String, String>) {
    if notes.is_empty() {
        return;
    }
    let evidence = report
        .dependencies
        .iter_mut()
        .flat_map(|d| d.evidence.iter_mut())
        .chain(
            report
                .datastores
                .nodes
                .iter_mut()
                .flat_map(|n| n.evidence.iter_mut()),
        )
        .chain(
            report
                .datastores
                .usages
                .iter_mut()
                .flat_map(|u| u.evidence.iter_mut()),
        );
    for evidence in evidence {
        if let Some(note) = notes.get(&evidence.file)
            && !evidence.description.ends_with(note.as_str())
        {
            evidence.description = format!("{} {note}", evidence.description);
        }
    }
}
