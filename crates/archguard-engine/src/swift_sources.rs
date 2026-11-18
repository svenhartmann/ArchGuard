//! Swift Package Manager targets of the walked sources (ADR 0018). Every
//! walked `Package.swift` is read as text, never run; Xcode projects are
//! not read, so their files belong to no target.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_extract::{Lang, SourceFile};

/// A package target: its name and its root-relative directory.
pub(crate) type SwiftTarget = (String, String);

/// The literal targets of every walked `Package.swift`. An unreadable
/// manifest declares nothing.
pub(crate) fn targets(root: &Path, files: &[SourceFile]) -> Vec<SwiftTarget> {
    let mut out = Vec::new();
    for file in files.iter().filter(|f| f.lang == Lang::Swift) {
        let (dir, name) = file
            .rel_path
            .rsplit_once('/')
            .unwrap_or(("", &file.rel_path));
        if name != "Package.swift" {
            continue;
        }
        let Ok(content) = crate::project_files::read_manifest(root, Path::new(&file.rel_path))
        else {
            continue;
        };
        for target in archguard_extract::swift::parse_package_swift(&content) {
            let path = target.path.trim_start_matches("./");
            let dir = match dir {
                "" => path.to_string(),
                dir => format!("{dir}/{path}"),
            };
            out.push((target.name, dir));
        }
    }
    out
}

/// Swift target name → the stripped directories of that target that hold
/// scanned `(rel_path, stripped)` files. Two packages declaring one name
/// leave it ambiguous, so an `import` of it stays unresolved.
pub(crate) fn modules<'a>(
    root: &Path,
    files: &[SourceFile],
    scanned: impl Iterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut modules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut scanned = scanned.peekable();
    if scanned.peek().is_none() {
        return modules;
    }
    let targets = targets(root, files);
    for (rel_path, stripped) in scanned {
        let Some((name, dir)) = target_of(&targets, rel_path) else {
            continue;
        };
        // The target directory as the stripped paths spell it.
        let tail = rel_path.len() - dir.len();
        if let Some(dir) = stripped.get(..stripped.len().saturating_sub(tail)) {
            modules.entry(name.clone()).or_default().insert(dir.into());
        }
    }
    modules
}

/// The target a root-relative file belongs to: the deepest target
/// directory that contains it.
pub(crate) fn target_of<'t>(targets: &'t [SwiftTarget], rel_path: &str) -> Option<&'t SwiftTarget> {
    targets
        .iter()
        .filter(|(_, dir)| rel_path.starts_with(&format!("{dir}/")))
        .max_by_key(|(_, dir)| dir.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_come_from_every_walked_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "Package.swift",
            "let p = Package(targets: [.target(name: \"Core\"), .target(name: \"Api\", path: \"./Sources/Http\")])\n",
        );
        write("tools/Package.swift", ".executableTarget(name: \"Gen\")\n");
        write("Sources/Core/A.swift", "");
        let files = archguard_extract::walk(dir.path(), &[]).unwrap();
        let found = targets(dir.path(), &files);
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            found,
            [
                pair("Core", "Sources/Core"),
                pair("Api", "Sources/Http"),
                pair("Gen", "tools/Sources/Gen"),
            ]
        );
        assert_eq!(
            target_of(&found, "Sources/Core/Model/A.swift").map(|t| t.0.as_str()),
            Some("Core")
        );
        assert_eq!(target_of(&found, "Sources/CoreX/A.swift"), None);
    }
}
