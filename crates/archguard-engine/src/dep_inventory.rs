//! External dependency inventory (R22).
//!
//! Counts external dependencies from lockfiles and manifests found under
//! the project root: `Cargo.lock`, `package-lock.json`,
//! `requirements*.txt` / `pyproject.toml`, `composer.json`, `pom.xml`.
//! `version` is `Some` only where the file pins one (lockfiles, `==`
//! pins, explicit `<version>`); constraint-only manifests keep `None` —
//! a constraint is not a resolved version.
//!
//! Staleness v1 reads explicitly imported registry metadata from
//! `.archguard/dep-meta.json` (versioned schema, producer unverified
//! like every other import):
//!
//! ```json
//! {"version": 1, "packages": [{"ecosystem": "cargo", "name": "serde", "latest": "1.0.219"}]}
//! ```
//!
//! Without metadata every `stale` stays `None` and `stale_count` is
//! `None` — a missing registry lookup is never "fresh". Dependencies
//! without a version can never be marked stale either: there is nothing
//! to compare against.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The inventory types live in core (the `Graph` payload field needs
/// them there); re-exported so engine consumers keep a single import site.
pub use archguard_core::{DependencyInventory, ExternalDependency};

/// Ecosystems counted as dependencies of the project itself (a
/// requirement of one dependency is not an external dependency of us).
const PLATFORM_PREFIXES: &[&str] = &["php", "ext-"];

/// One entry of the imported `.archguard/dep-meta.json`.
#[derive(Debug, Clone, Deserialize)]
struct MetaEntry {
    ecosystem: String,
    name: String,
    latest: String,
}

#[derive(Debug, Deserialize)]
struct MetaFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    packages: Vec<MetaEntry>,
}

const META_REL_PATH: &str = ".archguard/dep-meta.json";

/// Directories that never contain first-party manifests.
const SKIP_DIRS: &[&str] = &[
    ".archguard",
    ".git",
    ".hg",
    ".venv",
    "build",
    "dist",
    "node_modules",
    "out",
    "target",
    "vendor",
    "venv",
];

/// Find every manifest below `root` (bounded depth, build artifacts and
/// dependency caches skipped), deterministic order by path.
fn find_manifests(root: &Path) -> Vec<(String, PathBuf)> {
    fn known(name: &str) -> Option<&'static str> {
        match name {
            "Cargo.lock" => Some("cargo"),
            "package-lock.json" => Some("npm"),
            "pyproject.toml" => Some("pypi"),
            "composer.json" => Some("composer"),
            "pom.xml" => Some("maven"),
            other if other.starts_with("requirements") && other.ends_with(".txt") => Some("pypi"),
            _ => None,
        }
    }
    fn walk(dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
        if depth > 4 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if !SKIP_DIRS.contains(&name) {
                    dirs.push(path);
                }
            } else if let Some(eco) = known(name) {
                out.push((eco.to_string(), path));
            }
        }
        dirs.sort();
        for sub in dirs {
            walk(&sub, depth + 1, out);
        }
    }
    let mut found = Vec::new();
    walk(root, 0, &mut found);
    found.sort();
    found
}

/// Collect `(name, version)` pairs from one `Cargo.lock`.
fn cargo_lock_deps(text: &str) -> Vec<(String, Option<String>)> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    table
        .get("package")
        .and_then(toml::Value::as_array)
        .map(|packages| {
            packages
                .iter()
                .filter_map(|package| {
                    let name = package.get("name")?.as_str()?.to_string();
                    let version = package
                        .get("version")
                        .and_then(toml::Value::as_str)
                        .map(str::to_string);
                    Some((name, version))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Collect `(name, version)` pairs from a `package-lock.json` (v2/v3
/// `packages` map preferred, v1 `dependencies` as fallback). The root
/// package entry (`""`) is not a dependency of the project.
fn package_lock_deps(text: &str) -> Vec<(String, Option<String>)> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    if let Some(packages) = root.get("packages").and_then(serde_json::Value::as_object) {
        for (key, entry) in packages {
            // v3 keys are paths; only `…/node_modules/<name>` entries are
            // third-party — the root `""` and workspace entries are us.
            let Some((_, tail)) = key.rsplit_once("node_modules/") else {
                continue;
            };
            if tail.is_empty() {
                continue;
            }
            let version = entry.get("version").and_then(serde_json::Value::as_str);
            deps.push((tail.to_string(), version.map(str::to_string)));
        }
    } else if let Some(dependencies) = root
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
    {
        for (name, entry) in dependencies {
            let version = entry.get("version").and_then(serde_json::Value::as_str);
            deps.push((name.clone(), version.map(str::to_string)));
        }
    }
    deps
}

/// Split a PEP 508 requirement like `requests[oauth]>=2.28,<3` into
/// name and (for `==` pins) exact version. Constraints are not resolved
/// versions, so only `==` yields `Some`.
fn requirement_split(line: &str) -> Option<(String, Option<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
        return None;
    }
    let name_end = line
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.')
        .unwrap_or(line.len());
    let name = &line[..name_end];
    if name.is_empty() {
        return None;
    }
    let rest = line[name_end..].trim_start();
    let version = rest
        .strip_prefix("==")
        .map(|tail| {
            tail.split([',', ';', ' ', '#'])
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|v| !v.is_empty());
    Some((name.to_string(), version))
}

fn requirements_txt_deps(text: &str) -> Vec<(String, Option<String>)> {
    text.lines().filter_map(requirement_split).collect()
}

fn pyproject_deps(text: &str) -> Vec<(String, Option<String>)> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    table
        .get("project")
        .and_then(|project| project.get("dependencies"))
        .and_then(toml::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(toml::Value::as_str)
                .filter_map(requirement_split)
                .collect()
        })
        .unwrap_or_default()
}

/// `composer.json` `require` map; platform requirements (`php`, `ext-*`)
/// are environment facts, not dependencies. Constraints are not
/// resolved versions → `version: None`.
fn composer_deps(text: &str) -> Vec<(String, Option<String>)> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    root.get("require")
        .and_then(serde_json::Value::as_object)
        .map(|require| {
            require
                .keys()
                .filter(|name| !PLATFORM_PREFIXES.iter().any(|p| name.starts_with(p)))
                .map(|name| (name.clone(), None))
                .collect()
        })
        .unwrap_or_default()
}

/// `pom.xml` `<dependencies>/<dependency>` entries. `dependencyManagement`
/// and plugin dependencies are excluded — they do not ship with the
/// artifact.
fn pom_deps(text: &str) -> Vec<(String, Option<String>)> {
    let Ok(document) = roxmltree::Document::parse(text) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    // `root()` is the document node; the <project>/<dependencies>
    // elements are among its descendants.
    for dependencies in document
        .descendants()
        .filter(|n| n.has_tag_name("dependencies"))
    {
        for dependency in dependencies
            .children()
            .filter(|n| n.has_tag_name("dependency"))
        {
            let child = |tag: &str| {
                dependency
                    .children()
                    .find(|n| n.has_tag_name(tag))
                    .and_then(|n| n.text())
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
            };
            let (Some(group), Some(artifact)) = (child("groupId"), child("artifactId")) else {
                continue;
            };
            deps.push((format!("{group}:{artifact}"), child("version")));
        }
    }
    deps
}

fn deps_for(eco: &str, path: &Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match (eco, name) {
        ("cargo", _) => cargo_lock_deps(&text),
        ("npm", _) => package_lock_deps(&text),
        ("pypi", file) if file.starts_with("requirements") => requirements_txt_deps(&text),
        ("pypi", _) => pyproject_deps(&text),
        ("composer", _) => composer_deps(&text),
        ("maven", _) => pom_deps(&text),
        _ => Vec::new(),
    }
}

/// Read the imported registry metadata; `None` when absent. A corrupt
/// file is reported in `notes` and treated as absent — staleness stays
/// unknown, the inventory itself does not fail.
fn load_meta(root: &Path, notes: &mut Vec<String>) -> Option<BTreeMap<(String, String), String>> {
    let path = root.join(META_REL_PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            notes.push(format!(
                "{META_REL_PATH}: unreadable ({err}): staleness unknown"
            ));
            return None;
        }
    };
    match serde_json::from_str::<MetaFile>(&text) {
        Ok(meta) if meta.version == 1 => Some(
            meta.packages
                .into_iter()
                .map(|entry| ((entry.ecosystem, entry.name), entry.latest))
                .collect(),
        ),
        Ok(meta) => {
            notes.push(format!(
                "{META_REL_PATH}: unsupported schema version {} (expected 1): staleness unknown",
                meta.version
            ));
            None
        }
        Err(err) => {
            notes.push(format!(
                "{META_REL_PATH}: corrupt ({err}): fix or remove the file, staleness unknown"
            ));
            None
        }
    }
}

/// Build the inventory from the lockfiles and manifests below `root`.
/// Deterministic: dependencies sorted, notes in manifest order.
pub fn dependency_inventory(root: &Path) -> DependencyInventory {
    let mut notes = Vec::new();
    let mut seen: BTreeSet<(String, String, Option<String>)> = BTreeSet::new();

    for (eco, path) in find_manifests(root) {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let entries = deps_for(&eco, &path);
        if entries.is_empty() {
            notes.push(format!(
                "{relative}: no dependencies counted (empty or unparseable)"
            ));
            continue;
        }
        for (name, version) in entries {
            seen.insert((eco.clone(), name, version));
        }
        notes.push(format!("{relative}: {eco}"));
    }
    // One pass over the deduplicated set: the same package pinned in two
    // lockfiles counts once, `total` and `by_ecosystem` cannot drift.
    let mut by_ecosystem: BTreeMap<String, usize> = BTreeMap::new();
    for (eco, _, _) in &seen {
        *by_ecosystem.entry(eco.clone()).or_default() += 1;
    }
    let mut dependencies: Vec<ExternalDependency> = Vec::new();
    let meta = load_meta(root, &mut notes);
    for (ecosystem, name, version) in &seen {
        let stale = meta.as_ref().and_then(|meta| {
            let latest = meta.get(&(ecosystem.clone(), name.clone()))?;
            version.as_ref().map(|version| version != latest)
        });
        dependencies.push(ExternalDependency {
            ecosystem: ecosystem.clone(),
            name: name.clone(),
            version: version.clone(),
            stale,
        });
    }
    if seen.is_empty() {
        notes.push(
            "no supported manifest or lockfile found (cargo, npm, pypi, composer, maven)"
                .to_string(),
        );
    }
    let stale_count = meta.as_ref().map(|_| {
        dependencies
            .iter()
            .filter(|dependency| dependency.stale == Some(true))
            .count()
    });
    dependencies.sort_by(|a, b| {
        (&a.ecosystem, &a.name, a.version.as_deref()).cmp(&(
            &b.ecosystem,
            &b.name,
            b.version.as_deref(),
        ))
    });
    DependencyInventory {
        total: dependencies.len(),
        by_ecosystem,
        stale_count,
        dependencies,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, content).expect("write");
    }

    #[test]
    fn cargo_lock_pins_are_counted() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "Cargo.lock",
            r#"version = 3

[[package]]
name = "archguard-core"
version = "0.1.0"

[[package]]
name = "serde"
version = "1.0.219"
"#,
        );
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.total, 2);
        assert_eq!(inventory.by_ecosystem["cargo"], 2);
        assert_eq!(inventory.dependencies[1].name, "serde");
        assert_eq!(
            inventory.dependencies[1].version.as_deref(),
            Some("1.0.219")
        );
        assert_eq!(inventory.stale_count, None, "no metadata imported");
    }

    #[test]
    fn npm_package_lock_v3_skips_the_root_package() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "apps/web/package-lock.json",
            r#"{"packages":{"":{"name":"own-app"},"node_modules/left-pad":{"version":"1.3.9"},"node_modules/@scope/pkg":{"version":"2.0.0"}}}"#,
        );
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.total, 2);
        assert_eq!(inventory.by_ecosystem["npm"], 2);
        let names: Vec<&str> = inventory
            .dependencies
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(names, ["@scope/pkg", "left-pad"]);
    }

    #[test]
    fn npm_package_lock_v1_falls_back_to_dependencies() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "package-lock.json",
            r#"{"dependencies":{"express":{"version":"4.19.2"}}}"#,
        );
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.dependencies[0].name, "express");
        assert_eq!(inventory.dependencies[0].version.as_deref(), Some("4.19.2"));
    }

    #[test]
    fn pypi_pins_have_versions_constraints_do_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "requirements.txt",
            "# comment\nrequests==2.32.3\nflask>=3.0\n-r other.txt\n",
        );
        write(
            dir.path(),
            "pyproject.toml",
            "[project]\ndependencies = [\"click>=8.1\"]\n",
        );
        let inventory = dependency_inventory(dir.path());
        let by_name: BTreeMap<&str, Option<&str>> = inventory
            .dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.version.as_deref()))
            .collect();
        assert_eq!(by_name["requests"], Some("2.32.3"));
        assert_eq!(
            by_name["flask"], None,
            "a constraint is not a resolved version"
        );
        assert_eq!(by_name["click"], None);
        assert_eq!(inventory.by_ecosystem["pypi"], 3);
    }

    #[test]
    fn composer_skips_platform_requirements() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "composer.json",
            r#"{"require":{"php":">=8.2","ext-json":"*","monolog/monolog":"^3.0"}}"#,
        );
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.dependencies.len(), 1);
        assert_eq!(inventory.dependencies[0].name, "monolog/monolog");
    }

    #[test]
    fn maven_counts_coordinate_pairs_with_explicit_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "pom.xml",
            r#"<project><dependencies>
<dependency><groupId>org.slf4j</groupId><artifactId>slf4j-api</artifactId><version>2.0.13</version></dependency>
<dependency><groupId>a</groupId><artifactId>b</artifactId></dependency>
</dependencies></project>"#,
        );
        let inventory = dependency_inventory(dir.path());
        // Sorted by (ecosystem, name): the coordinate with the smaller
        // group id leads, and its missing <version> stays unknown.
        assert_eq!(inventory.dependencies[0].name, "a:b");
        assert_eq!(inventory.dependencies[0].version, None);
        assert_eq!(inventory.dependencies[1].name, "org.slf4j:slf4j-api");
        assert_eq!(inventory.dependencies[1].version.as_deref(), Some("2.0.13"));
    }

    #[test]
    fn pom_deps_finds_direct_dependencies() {
        let deps = pom_deps(
            r#"<project><dependencies>
<dependency><groupId>g</groupId><artifactId>a</artifactId><version>1.0</version></dependency>
</dependencies></project>"#,
        );
        assert_eq!(deps.len(), 1, "{deps:?}");
    }

    #[test]
    fn unknown_project_is_empty_with_a_coverage_note() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "Gemfile", "gem 'rails'");
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.total, 0);
        assert!(
            inventory
                .notes
                .iter()
                .any(|note| note.contains("no supported manifest")),
            "{:?}",
            inventory.notes
        );
    }

    #[test]
    fn imported_meta_marks_stale_and_fresh_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "Cargo.lock",
            "[[package]]\nname = \"serde\"\nversion = \"1.0.100\"\n\n[[package]]\nname = \"toml\"\nversion = \"0.8.19\"\n",
        );
        write(
            dir.path(),
            ".archguard/dep-meta.json",
            r#"{"version":1,"packages":[{"ecosystem":"cargo","name":"serde","latest":"1.0.219"},{"ecosystem":"cargo","name":"toml","latest":"0.8.19"}]}"#,
        );
        let inventory = dependency_inventory(dir.path());
        let stale: Vec<(&str, Option<bool>)> = inventory
            .dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.stale))
            .collect();
        assert_eq!(stale, [("serde", Some(true)), ("toml", Some(false))]);
        assert_eq!(inventory.stale_count, Some(1));
    }

    #[test]
    fn corrupt_meta_leaves_staleness_unknown_with_a_note() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "Cargo.lock",
            "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
        );
        write(dir.path(), ".archguard/dep-meta.json", "{ not json");
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.total, 1);
        assert_eq!(inventory.stale_count, None);
        assert_eq!(inventory.dependencies[0].stale, None);
        assert!(
            inventory.notes.iter().any(|note| note.contains("corrupt")),
            "{:?}",
            inventory.notes
        );
    }

    #[test]
    fn same_package_in_two_lockfiles_counts_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n";
        write(dir.path(), "Cargo.lock", lock);
        write(dir.path(), "sub-crate/Cargo.lock", lock);
        let inventory = dependency_inventory(dir.path());
        assert_eq!(inventory.total, 1);
        assert_eq!(inventory.by_ecosystem["cargo"], 1);
    }

    #[test]
    fn default_is_not_measured_not_clean() {
        let inventory = DependencyInventory::default();
        assert_eq!(inventory.total, 0);
        assert_eq!(inventory.stale_count, None, "staleness unknown, not zero");
    }

    #[test]
    fn signature_is_stable() {
        let _: fn(&Path) -> DependencyInventory = dependency_inventory;
    }
}
