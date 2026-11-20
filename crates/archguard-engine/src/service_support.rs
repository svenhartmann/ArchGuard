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
pub(crate) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
/// Test code: a test directory, a .NET test project (`Shop.Tests`, `Shop.UnitTests`),
/// a Gradle test source set (`integrationTest`, `testFixtures`), a pytest file
/// (`test_x.py`, `x_test.py`) or a `.test.`/`.spec.` file. The camel-case checks are
/// case-sensitive so `Contest`, `latest` and `Testimonials` stay production code.
pub(crate) fn test_path(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    rel.split('/').any(|p| {
        let tail = p.rsplit('.').next().unwrap_or(p);
        let lower = p.to_ascii_lowercase();
        matches!(lower.as_str(), "test" | "tests" | "__tests__" | "__mocks__")
            || lower.ends_with(".tests")
            || lower.ends_with(".test")
            || p.ends_with("Tests")
            // Singular `Test` only for camelCase Gradle source sets (`integrationTest`)
            // and dotted projects (`Shop.Test`), so a feature folder like `ABTest/` stays code.
            || (tail.ends_with("Test") && (p.contains('.') || tail.starts_with(|c: char| c.is_ascii_lowercase())))
            || [p, tail].iter().any(|s| {
                s.strip_prefix("test")
                    .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase()))
            })
    }) || file.starts_with("test_")
        || file.ends_with("_test.py")
        || rel.contains(".test.")
        || rel.contains(".spec.")
}
/// The services whose source scope (build context or project path) is the
/// deepest one containing `rel`; several when their scopes tie.
pub(crate) fn owners(topology: &ServiceTopology, rel: &str) -> Vec<String> {
    let mut best: Option<(usize, std::collections::BTreeSet<&str>)> = None;
    for s in &topology.services {
        let project = s
            .project
            .as_ref()
            .and_then(|id| topology.projects.iter().find(|p| p.id == *id))
            .map(|p| p.path.as_str());
        for dir in s.build_context.as_deref().into_iter().chain(project) {
            let dir = dir.trim_end_matches('/');
            let depth = if dir == "." || dir.is_empty() {
                0
            } else if rel.starts_with(&format!("{dir}/")) {
                dir.split('/').count()
            } else {
                continue;
            };
            match &mut best {
                Some((d, ids)) if *d == depth => {
                    ids.insert(&s.id);
                }
                Some((d, _)) if *d > depth => {}
                _ => best = Some((depth, std::collections::BTreeSet::from([s.id.as_str()]))),
            }
        }
    }
    best.map_or(vec![], |(_, ids)| {
        ids.into_iter().map(str::to_owned).collect()
    })
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
    excludes: archguard_extract::Excludes,
}
impl Discovery {
    pub(crate) fn new(root: &Path, report: &mut ServiceTopology) -> Result<Self, ArchguardError> {
        // Only explicit discovery exclusions apply here. Import defaults (e.g.
        // tests/**), source roots and semantic settings do not constrain inventory.
        let mut patterns = Vec::new();
        for name in [crate::config::DEFAULT_CONFIG_PATH, "archguard.toml"] {
            // No-follow read: a symlinked config is refused (as in config.rs), so
            // a parse error can never quote a file outside the root.
            if std::fs::symlink_metadata(root.join(name)).is_err() {
                continue;
            }
            match crate::project_files::read_manifest(root, Path::new(name))
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
        Ok(Self {
            excludes: archguard_extract::Excludes::new(&patterns)?,
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
                    && !excludes
                        .is_match(&path, entry.file_type().is_some_and(|kind| kind.is_dir()))
            })
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn symlinked_discovery_config_is_not_read_or_quoted() {
        let outside = tempfile::tempdir().expect("tempdir");
        let secret = outside.path().join("secret");
        std::fs::write(&secret, "TOPSECRET_TOKEN = = abc\n").expect("write");
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(root.path().join(".archguard")).expect("mkdir");
        std::os::unix::fs::symlink(&secret, root.path().join(".archguard/archguard.toml"))
            .expect("symlink");
        let mut report = ServiceTopology::default();
        Discovery::new(root.path(), &mut report).expect("discovery");
        let rendered = format!("{report:?}");
        assert!(!rendered.contains("TOPSECRET"), "{rendered}");
        assert_eq!(report.diagnostics.len(), 1, "{rendered}");

        // A regular config still applies its exclusions.
        std::fs::remove_file(root.path().join(".archguard/archguard.toml")).expect("rm");
        std::fs::write(
            root.path().join("archguard.toml"),
            "[project]\nexclude = [\"skipped/**\"]\n",
        )
        .expect("write");
        let mut report = ServiceTopology::default();
        let discovery = Discovery::new(root.path(), &mut report).expect("discovery");
        assert!(report.diagnostics.is_empty(), "{report:?}");
        std::fs::create_dir(root.path().join("skipped")).expect("mkdir");
        std::fs::write(root.path().join("skipped/x.txt"), "").expect("write");
        let walked: Vec<_> = discovery
            .walk(root.path())
            .flatten()
            .map(|e| relative(root.path(), e.path()))
            .collect();
        assert!(
            !walked.iter().any(|p| p.starts_with("skipped")),
            "{walked:?}"
        );
    }
}
