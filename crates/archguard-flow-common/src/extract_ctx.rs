//! What every UI-flow and domain extractor (ADR 0021, 0022) gets and shares:
//! the context of one run (root and configuration), file discovery, reading
//! with a size limit, and the folder rules for owners and application names.
use std::path::Path;

use std::collections::BTreeMap;

use archguard_core::{
    AccessMeaning, AccessState, Condition, Config, DiagnosticSeverity, ServiceTopology, UiFlow,
    UiFlowDiagnostic,
};

use crate::lang_ts::{Item, Value};
use crate::repo_file::read_repo_file;
use crate::support::{Discovery, excluded, relative, test_path};
use crate::text_decode::{Decoded, decode_text};

/// One extraction run: the analysed root and the configuration in force.
pub struct ExtractContext<'a> {
    pub root: &'a Path,
    pub config: &'a Config,
    /// Why the root's own configuration was not read, when it was looked up
    /// by [`with_root_config`]; extractors that use the configuration note it.
    pub config_error: Option<String>,
    /// When set, `[views] backends` outside this canonical root are not read
    /// (the MCP server root: no path from a config may leave it). The CLI
    /// leaves it unset: there the user wrote the path.
    pub allowed_root: Option<&'a Path>,
    /// The configuration file in use, as named in diagnostics; when unset
    /// the one `root` holds is named.
    pub config_file: Option<String>,
}

impl<'a> ExtractContext<'a> {
    /// A run with a configuration the caller has read (e.g. `--config`).
    pub fn new(root: &'a Path, config: &'a Config) -> Self {
        Self {
            root,
            config,
            config_error: None,
            allowed_root: None,
            config_file: None,
        }
    }

    /// [`Self::allowed_root`]: backends outside `root` are not read.
    pub fn confined_to(mut self, root: &'a Path) -> Self {
        self.allowed_root = Some(root);
        self
    }

    /// [`Self::config_file`], e.g. the path given by `--config`.
    pub fn with_config_file(mut self, file: impl Into<String>) -> Self {
        self.config_file = Some(file.into());
        self
    }
}

/// Runs `run` with the configuration found below `root`, the default one when
/// there is none or it does not parse (then `config_error` says why).
pub fn with_root_config<T>(root: &Path, run: impl FnOnce(&ExtractContext) -> T) -> T {
    let (config, config_error) = match root_config(root) {
        Ok(config) => (config, None),
        Err(e) => (Config::default(), Some(e)),
    };
    run(&ExtractContext {
        root,
        config: &config,
        config_error,
        allowed_root: None,
        config_file: None,
    })
}

/// The configuration `load_config` would discover: `.archguard/archguard.toml`,
/// then the legacy `archguard.toml`, without following links. Not through
/// `crate::config`: that module reaches `check`, `check` reaches the
/// extractors, and the architecture gate refuses the cycle.
pub fn root_config(root: &Path) -> Result<Config, String> {
    match found_root_config(root) {
        Ok(found) => Ok(found.unwrap_or_default()),
        Err((rel, message)) => Err(format!("{rel}: {message}")),
    }
}

/// The configuration files [`root_config`] looks for, in order.
const CONFIG_FILES: [&str; 2] = [".archguard/archguard.toml", "archguard.toml"];

/// [`root_config`], `None` when `root` has no configuration file; an error
/// names the file (relative to `root`) and why it was not read.
pub fn found_root_config(root: &Path) -> Result<Option<Config>, (String, String)> {
    for rel in CONFIG_FILES {
        let fail = |e: &dyn std::fmt::Display| (rel.to_string(), e.to_string());
        match read_repo_file(root, Path::new(rel), 1 << 20) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes).map_err(|e| fail(&e))?;
                return toml::from_str::<Config>(&text)
                    .map(Some)
                    .map_err(|e| fail(&e));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(fail(&e)),
        }
    }
    Ok(None)
}

/// The configuration file `root` holds, relative to it (for diagnostics):
/// the one [`root_config`] reads, else the legacy name.
pub fn config_file_in(root: &Path) -> &'static str {
    (CONFIG_FILES.into_iter())
        .find(|rel| std::fs::symlink_metadata(root.join(rel)).is_ok())
        .unwrap_or(CONFIG_FILES[1])
}

/// The regular files below `root` in walk order, as paths relative to it,
/// honouring the discovery exclusions.
pub struct Walk {
    pub files: Vec<String>,
    /// Directory entries that could not be read.
    pub errors: Vec<String>,
    /// Notes of the discovery settings: (file, message).
    pub notes: Vec<(String, String)>,
}

/// [`Walk`] below `root`; `Err` when the discovery settings cannot be read.
pub fn walk(root: &Path) -> Result<Walk, String> {
    let mut scratch = ServiceTopology::default();
    let discovery = Discovery::new(root, &mut scratch).map_err(|e| e.to_string())?;
    let (mut files, mut errors) = (Vec::new(), Vec::new());
    for entry in discovery.walk(root) {
        match entry {
            Ok(e) if e.file_type().is_some_and(|t| t.is_file()) => {
                files.push(relative(root, e.path()));
            }
            Ok(_) => {}
            Err(e) => errors.push(e.to_string()),
        }
    }
    let notes = (scratch.diagnostics.into_iter())
        .map(|d| (d.file, d.message))
        .collect();
    Ok(Walk {
        files,
        errors,
        notes,
    })
}

/// The walk of the domain extractors: hidden files included, no ignore files,
/// the engine's excluded directories and `skip` left out, at most 64 levels.
pub fn domain_walk(root: &Path, skip: &'static [&'static str]) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .git_global(false)
        .max_depth(Some(64))
        .filter_entry(move |e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !(excluded(&name) || skip.contains(&name.as_ref()))
        })
        .build()
}

/// A file (`""` for none) and why it was not read.
pub type Gap = (String, String);

/// The sources of one domain scan (ADR 0022): the files of [`domain_walk`]
/// that `keep` accepts, test files left out, sorted, at most 5,000, each read
/// up to 1 MiB. Returns (path, text) pairs and the gaps as (file, message);
/// `language` names the files in the message about the cap.
pub fn domain_sources(
    root: &Path,
    skip: &'static [&'static str],
    keep: impl Fn(&str) -> bool,
    language: &str,
) -> (Vec<(String, String)>, Vec<Gap>) {
    const MAX_FILES: usize = 5_000;
    let (mut paths, mut gaps) = (Vec::new(), Vec::new());
    for entry in domain_walk(root, skip) {
        match entry {
            Ok(e) if e.file_type().is_some_and(|t| t.is_file()) => {
                let rel = relative(root, e.path());
                if keep(&rel) && !test_path(&rel) {
                    paths.push(rel);
                }
            }
            Ok(_) => {}
            Err(e) => gaps.push((String::new(), format!("Directory not read: {e}"))),
        }
    }
    paths.sort();
    if let Some(first) = cap(&mut paths, MAX_FILES) {
        let message = format!(
            "Domain scan stopped after {MAX_FILES} {language} files; later files are unknown."
        );
        gaps.push((first, message));
    }
    let mut sources = Vec::new();
    for rel in paths {
        match read_text(root, &rel, 1 << 20) {
            Ok(text) => sources.push((rel, text)),
            Err(_) => gaps.push((
                rel,
                "Source file not read: larger than 1 MiB, not a regular file or not readable; \
                 its domain elements are unknown."
                    .into(),
            )),
        }
    }
    (sources, gaps)
}

/// Keeps the first `max` paths; the first one left out, if any.
pub fn cap(paths: &mut Vec<String>, max: usize) -> Option<String> {
    let dropped = paths.get(max).cloned();
    paths.truncate(max);
    dropped
}

/// A file below `root` as text: at most `max` bytes, decoded by
/// [`decode_text`]. Invalid UTF-8 is read lossily (identifiers are usually
/// ASCII and survive); bytes that give no text at all (broken UTF-16) are an
/// `InvalidData` error, so every caller's gap path reports them.
// shortcut: lossily decoded UTF-8 is read without a per-file diagnostic; upgrade when the
// ctx gets a diagnostic channel at this call (read_text is a free function without one).
pub fn read_text(root: &Path, rel: &str, max: u64) -> std::io::Result<String> {
    let bytes = read_repo_file(root, Path::new(rel), max)?;
    match decode_text(&bytes) {
        Decoded::Clean(text) | Decoded::Lossy(text) => Ok(text),
        Decoded::Unreadable => Err(invalid("UTF-16 with a broken byte sequence")),
    }
}

fn invalid(why: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_owned())
}

/// A web front-end source up to 1 MiB ([`read_text`]); the gap message when
/// it cannot be read.
pub fn read_source(root: &Path, rel: &str) -> Result<String, String> {
    read_text(root, rel, 1 << 20).map_err(|e| {
        format!(
            "File not read ({e}): larger than 1 MiB, not a regular file or not readable; \
             its routes, links and calls are unknown."
        )
    })
}

/// A gap at `file:line`: something is unknown, so the flow is not complete.
pub fn gap(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: impl Into<String>) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Gap,
    });
}

/// Sorts and deduplicates every list of an extracted flow; it is complete
/// when no diagnostic is a gap.
pub fn finish(flow: &mut UiFlow) {
    flow.screens.sort();
    flow.screens.dedup();
    flow.transitions.sort();
    flow.transitions.dedup();
    flow.actions.sort();
    flow.actions.dedup();
    flow.endpoints.sort();
    flow.endpoints.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow.complete = !(flow.diagnostics.iter()).any(|d| d.severity == DiagnosticSeverity::Gap);
}

/// The access state of check or guard `name` called with `args` under
/// `[views.access]`: `Protected(custom(name))` when it is not listed, `Open`
/// for `open` (always true); `$arg` forms take the string literal arguments
/// (several are `any`). `Err(name)` when such a form has no argument or a
/// non-literal one, or a fallback form is still unresolved. This is the
/// SvelteKit copy; other extractors may read a missing argument differently.
pub fn access_condition(
    access: &BTreeMap<String, AccessMeaning>,
    name: &str,
    args: &[Item],
) -> Result<AccessState, String> {
    use AccessState::Protected as P;
    let make: fn(String) -> Condition =
        match (access.get(name)).map(|m| m.at_use_site(!args.is_empty())) {
            None => return Ok(P(Condition::Custom(name.into()))),
            Some(AccessMeaning::Open) => return Ok(AccessState::Open),
            Some(AccessMeaning::Authenticated) => return Ok(P(Condition::Authenticated)),
            Some(AccessMeaning::Never) => return Ok(P(Condition::Never)),
            Some(AccessMeaning::Role(n)) => return Ok(P(Condition::Role(n.clone()))),
            Some(AccessMeaning::Policy(n)) => return Ok(P(Condition::Policy(n.clone()))),
            Some(AccessMeaning::RoleFromArguments) => Condition::Role,
            Some(AccessMeaning::PolicyFromArguments) => Condition::Policy,
            Some(AccessMeaning::FromArgumentsOr(..)) => return Err(name.into()),
        };
    let names: Option<Vec<Condition>> = (args.iter())
        .map(|a| match &a.value {
            Value::Str(s) => Some(make(s.clone())),
            _ => None,
        })
        .collect();
    match names {
        Some(mut names) if names.len() == 1 => Ok(P(names.remove(0))),
        Some(names) if !names.is_empty() => Ok(P(Condition::Any(names))),
        _ => Err(name.into()),
    }
}

/// The folders holding `rel`, nearest first, ending with the root `""`.
pub fn ancestors(rel: &str) -> impl Iterator<Item = &str> {
    let mut dir = Some(rel);
    std::iter::from_fn(move || {
        let parent = dir?.rsplit_once('/').map_or("", |(d, _)| d);
        dir = (!parent.is_empty()).then_some(parent);
        Some(parent)
    })
}

/// The name of folder `dir` below `root`: its last segment, for the root
/// itself the root's own folder name (`""` when it has none).
pub fn dir_name(root: &Path, dir: &str) -> String {
    match dir.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => (root.canonicalize().ok())
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default(),
    }
}

/// All of `items`, nested `all` flattened, sorted and deduplicated.
pub fn all(items: Vec<Condition>) -> Condition {
    let mut flat = Vec::new();
    for c in items {
        match c {
            Condition::All(inner) => flat.extend(inner),
            c => flat.push(c),
        }
    }
    flat.sort();
    flat.dedup();
    if flat.len() == 1 {
        flat.remove(0)
    } else {
        Condition::All(flat)
    }
}
