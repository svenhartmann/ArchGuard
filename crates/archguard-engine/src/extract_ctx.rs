//! What every UI-flow and domain extractor (ADR 0021, 0022) gets and shares:
//! the context of one run (root and configuration), file discovery, reading
//! with a size limit, and the folder rules for owners and application names.
use std::path::Path;

use archguard_core::{Config, DiagnosticSeverity, ServiceTopology, UiFlow, UiFlowDiagnostic};

use crate::project_files::read_repo_file;
use crate::services::support::{Discovery, excluded, relative, test_path};

/// One extraction run: the analysed root and the configuration in force.
pub struct ExtractContext<'a> {
    pub root: &'a Path,
    pub config: &'a Config,
    /// Why the root's own configuration was not read, when it was looked up
    /// by [`with_root_config`]; extractors that use the configuration note it.
    pub config_error: Option<String>,
}

impl<'a> ExtractContext<'a> {
    /// A run with a configuration the caller has read (e.g. `--config`).
    pub fn new(root: &'a Path, config: &'a Config) -> Self {
        Self {
            root,
            config,
            config_error: None,
        }
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
    })
}

/// The configuration `load_config` would discover: `.archguard/archguard.toml`,
/// then the legacy `archguard.toml`, without following links. Not through
/// `crate::config`: that module reaches `check`, `check` reaches the
/// extractors, and the architecture gate refuses the cycle.
pub fn root_config(root: &Path) -> Result<Config, String> {
    for rel in [".archguard/archguard.toml", "archguard.toml"] {
        match read_repo_file(root, Path::new(rel), 1 << 20) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes).map_err(|e| format!("{rel}: {e}"))?;
                return toml::from_str::<Config>(&text).map_err(|e| format!("{rel}: {e}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{rel}: {e}")),
        }
    }
    Ok(Config::default())
}

/// The regular files below `root` in walk order, as paths relative to it,
/// honouring the discovery exclusions.
pub(crate) struct Walk {
    pub(crate) files: Vec<String>,
    /// Directory entries that could not be read.
    pub(crate) errors: Vec<String>,
    /// Notes of the discovery settings: (file, message).
    pub(crate) notes: Vec<(String, String)>,
}

/// [`Walk`] below `root`; `Err` when the discovery settings cannot be read.
pub(crate) fn walk(root: &Path) -> Result<Walk, String> {
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
pub(crate) fn domain_walk(root: &Path, skip: &'static [&'static str]) -> ignore::Walk {
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

/// The sources of one domain scan (ADR 0022): the files of [`domain_walk`]
/// that `keep` accepts, test files left out, sorted, at most 5,000, each read
/// up to 1 MiB. Returns (path, text) pairs and the gaps as (file, message);
/// `language` names the files in the message about the cap.
pub(crate) fn domain_sources(
    root: &Path,
    skip: &'static [&'static str],
    keep: impl Fn(&str) -> bool,
    language: &str,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
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
pub(crate) fn cap(paths: &mut Vec<String>, max: usize) -> Option<String> {
    let dropped = paths.get(max).cloned();
    paths.truncate(max);
    dropped
}

/// A file below `root` as text: at most `max` bytes, a UTF-8 byte order mark
/// dropped, invalid UTF-8 replaced.
pub(crate) fn read_text(root: &Path, rel: &str, max: u64) -> std::io::Result<String> {
    let bytes = read_repo_file(root, Path::new(rel), max)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// A web front-end source up to 1 MiB ([`read_text`]); the gap message when
/// it cannot be read.
pub(crate) fn read_source(root: &Path, rel: &str) -> Result<String, String> {
    read_text(root, rel, 1 << 20).map_err(|e| {
        format!(
            "File not read ({e}): larger than 1 MiB, not a regular file or not readable; \
             its routes, links and calls are unknown."
        )
    })
}

/// A gap at `file:line`: something is unknown, so the flow is not complete.
pub(crate) fn gap(
    out: &mut Vec<UiFlowDiagnostic>,
    file: &str,
    line: u32,
    message: impl Into<String>,
) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Gap,
    });
}

/// Sorts and deduplicates every list of an extracted flow; it is complete
/// when no diagnostic is a gap.
pub(crate) fn finish(flow: &mut UiFlow) {
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

/// The folders holding `rel`, nearest first, ending with the root `""`.
pub(crate) fn ancestors(rel: &str) -> impl Iterator<Item = &str> {
    let mut dir = Some(rel);
    std::iter::from_fn(move || {
        let parent = dir?.rsplit_once('/').map_or("", |(d, _)| d);
        dir = (!parent.is_empty()).then_some(parent);
        Some(parent)
    })
}

/// The name of folder `dir` below `root`: its last segment, for the root
/// itself the root's own folder name (`""` when it has none).
pub(crate) fn dir_name(root: &Path, dir: &str) -> String {
    match dir.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => (root.canonicalize().ok())
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default(),
    }
}
