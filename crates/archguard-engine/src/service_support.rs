//! Helpers shared by the service topology scanners.
use std::path::Path;

use archguard_core::{ArchguardError, ServiceDiagnostic, ServiceTopology};

pub(super) fn diagnostic(
    report: &mut ServiceTopology,
    file: &str,
    service: Option<&str>,
    message: impl Into<String>,
) {
    report.complete = false;
    report.diagnostics.push(ServiceDiagnostic {
        file: file.into(),
        service: service.map(str::to_owned),
        message: message.into(),
    });
}
pub(super) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
pub(crate) fn excluded(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | ".archguard"
            | ".claude"
            | ".codex"
            | "examples"
            | "fixtures"
            | ".idea"
            | ".gradle"
            | "target"
            | "node_modules"
            | "vendor"
            | "dist"
            | "build"
            | ".venv"
            | "venv"
            | "__pycache__"
    )
}
/// Shared discovery bounds for manifests and deployment declarations.
/// Samples remain analyzable when explicitly selected as the analysis root.
pub(crate) struct Discovery {
    excludes: globset::GlobSet,
}
impl Discovery {
    pub(super) fn new(root: &Path, report: &mut ServiceTopology) -> Result<Self, ArchguardError> {
        // Only explicit discovery exclusions apply here. Import defaults (e.g.
        // tests/**), source roots and semantic settings do not constrain inventory.
        let mut patterns = Vec::new();
        for name in [crate::config::DEFAULT_CONFIG_PATH, "archguard.toml"] {
            let path = root.join(name);
            if !path.exists() {
                continue;
            }
            match std::fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|text| {
                    text.parse::<toml::Table>()
                        .map_err(|error| error.to_string())
                }) {
                Ok(table) => {
                    if let Some(excludes) = table.get("project").and_then(|p| p.get("exclude")) {
                        let entries = excludes.as_array().ok_or_else(|| {
                            ArchguardError::Config("project.exclude must be an array".into())
                        })?;
                        for entry in entries {
                            patterns.push(
                                entry
                                    .as_str()
                                    .ok_or_else(|| {
                                        ArchguardError::Config(
                                            "project.exclude entries must be strings".into(),
                                        )
                                    })?
                                    .to_owned(),
                            );
                        }
                    }
                }
                Err(error) => diagnostic(
                    report,
                    name,
                    None,
                    format!(
                        "Discovery exclusions could not be read; continuing independent service analysis: {error}"
                    ),
                ),
            }
            break;
        }
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in &patterns {
            builder.add(globset::Glob::new(pattern).map_err(|error| {
                ArchguardError::Config(format!("invalid discovery exclude: {error}"))
            })?);
        }
        Ok(Self {
            excludes: builder.build().map_err(|error| {
                ArchguardError::Config(format!("invalid discovery exclusions: {error}"))
            })?,
        })
    }
    pub(crate) fn walk(&self, root: &Path) -> ignore::Walk {
        let root = root.to_path_buf();
        let scope_root = root.clone();
        let excludes = self.excludes.clone();
        ignore::WalkBuilder::new(root)
            .hidden(false)
            .follow_links(false)
            .require_git(false)
            .git_global(false)
            .max_depth(Some(64))
            .filter_entry(move |entry| {
                if entry.depth() == 0 {
                    return true;
                }
                let name = entry.file_name().to_string_lossy();
                let path = relative(&scope_root, entry.path());
                !excluded(&name)
                    && !matches!(name.as_ref(), "bin" | "obj")
                    && !excludes.is_match(&path)
                    && !(entry.file_type().is_some_and(|kind| kind.is_dir())
                        && excludes.is_match(format!("{path}/")))
            })
            .build()
    }
}
