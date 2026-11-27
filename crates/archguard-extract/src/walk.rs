//! File discovery (ARCHITECTURE.md Kap. 6, step 2): walking the source tree.
//!
//! [`walk`] traverses a root directory with the `ignore` crate — which
//! respects `.gitignore` files even outside a git checkout — keeps only
//! files whose extension maps to a known [`Lang`], drops everything matched
//! by the caller's `exclude` globs (compiled with `globset`) and returns the
//! result sorted by `rel_path` so downstream output is deterministic.

use std::path::{Path, PathBuf};

use archguard_core::{ArchguardError, Excludes};
use ignore::WalkBuilder;

/// Programming language of a [`SourceFile`], detected by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    Rust,
    Ts,
    Js,
    Py,
    Java,
    CSharp,
    Php,
    Go,
    Kotlin,
    /// C and C++ share one language: a `.h` header can be either (ADR 0017).
    Cpp,
    Swift,
    Ruby,
    Scala,
}

impl Lang {
    /// The language a filename extension (without the dot) maps to,
    /// `None` for extensions archguard does not scan.
    pub fn from_extension(ext: &str) -> Option<Lang> {
        match ext {
            "rs" => Some(Lang::Rust),
            "ts" | "tsx" | "mts" | "cts" => Some(Lang::Ts),
            "js" | "jsx" | "mjs" | "cjs" => Some(Lang::Js),
            "py" => Some(Lang::Py),
            "java" => Some(Lang::Java),
            "cs" => Some(Lang::CSharp),
            "php" => Some(Lang::Php),
            "go" => Some(Lang::Go),
            // Not `.kts`: those are mostly Gradle build scripts, not application code.
            "kt" => Some(Lang::Kotlin),
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(Lang::Cpp),
            "swift" => Some(Lang::Swift),
            "rb" => Some(Lang::Ruby),
            // Not `.sc`: worksheets and Ammonite scripts, not application code.
            "scala" => Some(Lang::Scala),
            _ => None,
        }
    }

    /// The language of the path's extension, `None` if the extension is
    /// unknown or not valid UTF-8.
    fn from_path(path: &Path) -> Option<Lang> {
        Lang::from_extension(path.extension()?.to_str()?)
    }
}

/// One discovered source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Path of the file, rooted at the `root` passed to [`walk`].
    pub path: PathBuf,
    /// `root`-relative path with `/` separators on every platform; the
    /// canonical name used for sorting and (later) for graph nodes.
    pub rel_path: String,
    /// Language detected from the file extension.
    pub lang: Lang,
    /// `/`-joined `root`-relative directory of the nearest build manifest
    /// above the file ([`is_manifest`]), `""` for none or the root itself.
    /// Names files with a declared package apart per project in a monorepo.
    pub project: String,
}

/// Build manifests of the languages that name files after a declared
/// package or namespace (Java, Kotlin, Scala, C#, PHP).
fn is_manifest(name: &str) -> bool {
    matches!(
        name,
        "pom.xml" | "build.gradle" | "build.gradle.kts" | "build.sbt" | "composer.json"
    ) || [".csproj", ".fsproj", ".vbproj"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Collect every scannable source file under `root`.
///
/// The traversal respects `.gitignore` files and skips hidden entries
/// (the `ignore` crate defaults). Files whose extension maps to no [`Lang`]
/// are skipped, as are files whose [`SourceFile::rel_path`] is matched by any
/// of the `excludes` globs. The result is sorted by `rel_path`.
///
/// An invalid exclude glob yields [`ArchguardError::Config`]; filesystem
/// failures during the walk yield [`ArchguardError::Io`].
pub fn walk(root: &Path, excludes: &[String]) -> Result<Vec<SourceFile>, ArchguardError> {
    let excluded = Excludes::new(excludes)?;
    let walk_root = root.to_path_buf();
    let mut files = Vec::new();
    let mut projects = std::collections::BTreeSet::new();
    let walker = WalkBuilder::new(root)
        .require_git(false)
        .filter_entry(move |e| {
            e.depth() == 0
                || !excluded.is_match(
                    &relative_path(&walk_root, e.path()),
                    e.file_type().is_some_and(|t| t.is_dir()),
                )
        })
        .build();
    for entry in walker {
        let entry = entry.map_err(|e| ArchguardError::Io(std::io::Error::other(e.to_string())))?;
        // Depth 0 is `root` itself; only plain files can be sources (entries
        // with unknown file type, e.g. broken links, are skipped as well).
        if entry.depth() == 0 || !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if entry.depth() > 1 && entry.file_name().to_str().is_some_and(is_manifest) {
            let dir = relative_path(root, path.parent().unwrap_or(root));
            projects.insert(dir);
        }
        let Some(lang) = Lang::from_path(path) else {
            continue;
        };
        files.push(SourceFile {
            path: path.to_path_buf(),
            rel_path: relative_path(root, path),
            lang,
            project: String::new(),
        });
    }
    for file in &mut files {
        file.project = nearest_project(&file.rel_path, &projects);
    }
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(files)
}

/// The deepest of `projects` that contains `rel_path`, `""` for none.
fn nearest_project(rel_path: &str, projects: &std::collections::BTreeSet<String>) -> String {
    let mut dir = rel_path;
    while let Some((parent, _)) = dir.rsplit_once('/') {
        if projects.contains(parent) {
            return parent.to_string();
        }
        dir = parent;
    }
    String::new()
}

/// `path` relative to `root`, joined with `/` on every platform.
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create `dir/rel` (including parent directories) with empty content.
    fn touch(dir: &Path, rel: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
    }

    fn rel_paths(files: &[SourceFile]) -> Vec<&str> {
        files.iter().map(|f| f.rel_path.as_str()).collect()
    }

    fn walk_dir(dir: &Path, excludes: &[&str]) -> Result<Vec<SourceFile>, ArchguardError> {
        let owned: Vec<String> = excludes.iter().map(|s| (*s).to_string()).collect();
        walk(dir, &owned)
    }

    #[test]
    fn lang_from_extension_maps_every_documented_extension() {
        for (ext, lang) in [
            ("rs", Lang::Rust),
            ("ts", Lang::Ts),
            ("tsx", Lang::Ts),
            ("js", Lang::Js),
            ("jsx", Lang::Js),
            ("mjs", Lang::Js),
            ("py", Lang::Py),
            ("java", Lang::Java),
            ("cs", Lang::CSharp),
            ("php", Lang::Php),
            ("kt", Lang::Kotlin),
            ("swift", Lang::Swift),
            ("rb", Lang::Ruby),
            ("scala", Lang::Scala),
        ] {
            assert_eq!(Lang::from_extension(ext), Some(lang), "ext {ext}");
        }
        assert_eq!(Lang::from_extension("txt"), None);
        assert_eq!(Lang::from_extension("sc"), None);
        assert_eq!(Lang::from_extension("kts"), None);
        assert_eq!(Lang::from_extension(""), None);
    }

    #[test]
    fn walk_detects_languages_and_skips_unknown_extensions() {
        let dir = tempfile::tempdir().unwrap();
        for rel in [
            "a.rs", "b.ts", "c.tsx", "d.js", "e.jsx", "f.mjs", "g.py", "h.java", "i.cs", "j.php",
            "k.kt", "l.rb", "l.scala",
        ] {
            touch(dir.path(), rel);
        }
        touch(dir.path(), "notes.txt");
        touch(dir.path(), "Other.sc");
        touch(dir.path(), "noext");

        let files = walk_dir(dir.path(), &[]).unwrap();
        assert_eq!(
            rel_paths(&files),
            [
                "a.rs", "b.ts", "c.tsx", "d.js", "e.jsx", "f.mjs", "g.py", "h.java", "i.cs",
                "j.php", "k.kt", "l.rb", "l.scala",
            ]
        );
        let langs: Vec<Lang> = files.iter().map(|f| f.lang).collect();
        assert_eq!(
            langs,
            [
                Lang::Rust,
                Lang::Ts,
                Lang::Ts,
                Lang::Js,
                Lang::Js,
                Lang::Js,
                Lang::Py,
                Lang::Java,
                Lang::CSharp,
                Lang::Php,
                Lang::Kotlin,
                Lang::Ruby,
                Lang::Scala,
            ]
        );
    }

    #[test]
    fn an_sbt_build_names_its_project() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "svc/build.sbt");
        touch(dir.path(), "svc/src/main/scala/A.scala");
        let files = walk_dir(dir.path(), &[]).unwrap();
        assert_eq!(files[0].project, "svc");
    }

    #[test]
    fn rel_path_is_slash_separated_and_path_is_under_root() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "src/app/b.ts");
        touch(dir.path(), "src/a.py");

        let files = walk_dir(dir.path(), &[]).unwrap();
        assert_eq!(rel_paths(&files), ["src/a.py", "src/app/b.ts"]);
        for file in &files {
            assert!(file.path.starts_with(dir.path()));
            assert_eq!(file.path, dir.path().join(&file.rel_path));
        }
    }

    #[test]
    fn results_are_sorted_by_rel_path() {
        let dir = tempfile::tempdir().unwrap();
        for rel in ["z.rs", "m/n.py", "a/b/c.js", "a/x.java", "b.ts"] {
            touch(dir.path(), rel);
        }
        let files = walk_dir(dir.path(), &[]).unwrap();
        let paths = rel_paths(&files);
        assert_eq!(paths, ["a/b/c.js", "a/x.java", "b.ts", "m/n.py", "z.rs"]);
        let mut sorted = paths.to_vec();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn gitignore_is_respected_without_a_git_checkout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored.py\nvendor/\n").unwrap();
        touch(dir.path(), "ignored.py");
        touch(dir.path(), "kept.py");
        touch(dir.path(), "vendor/v.py");
        touch(dir.path(), "src/kept.rs");

        let files = walk_dir(dir.path(), &[]).unwrap();
        assert_eq!(rel_paths(&files), ["kept.py", "src/kept.rs"]);
    }

    #[test]
    fn exclude_globs_are_applied_to_rel_paths() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "src/main.rs");
        touch(dir.path(), "src/generated/gen.rs");
        touch(dir.path(), "src/deep/generated/g.py");
        touch(dir.path(), "app.spec.ts");
        touch(dir.path(), "src/app.spec.ts");
        touch(dir.path(), "src/app.ts");

        let files = walk_dir(dir.path(), &["**/generated/**", "**/*.spec.*"]).unwrap();
        assert_eq!(rel_paths(&files), ["src/app.ts", "src/main.rs"]);
    }

    /// An excluded directory is pruned, not entered: an unreadable
    /// directory inside it does not fail the walk.
    #[cfg(unix)]
    #[test]
    fn excluded_directories_are_not_entered() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "src/main.rs");
        touch(dir.path(), "vendor/lib/a.rs");
        let locked = dir.path().join("vendor/locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = walk_dir(dir.path(), &["vendor/**"]);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(rel_paths(&result.unwrap()), ["src/main.rs"]);
    }

    /// `dir` and `dir/` exclude a directory like `.gitignore` does, and a
    /// glob that matches a directory prunes it too: nothing below is
    /// walked, and an unreadable directory inside does not fail the walk.
    #[cfg(unix)]
    #[test]
    fn directory_patterns_exclude_and_prune_the_whole_directory() {
        use std::os::unix::fs::PermissionsExt;
        for (pattern, excluded_dir) in [
            ("vendor/", "vendor"),
            ("vendor", "vendor"),
            ("**/generated", "src/deep/generated"),
            ("src/generated", "src/generated"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            touch(dir.path(), "src/main.rs");
            touch(dir.path(), &format!("{excluded_dir}/a.rs"));
            touch(dir.path(), &format!("{excluded_dir}/lib/b.py"));
            let locked = dir.path().join(excluded_dir).join("locked");
            std::fs::create_dir_all(&locked).unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

            let result = walk_dir(dir.path(), &[pattern]);
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(rel_paths(&result.unwrap()), ["src/main.rs"], "{pattern}");
        }
    }

    /// A trailing `/` names a directory only; without it a file matches too.
    #[test]
    fn trailing_slash_never_matches_a_file() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "gen.rs");
        touch(dir.path(), "src/gen.rs/inner.rs");
        let files = walk_dir(dir.path(), &["gen.rs/"]).unwrap();
        assert_eq!(rel_paths(&files), ["gen.rs", "src/gen.rs/inner.rs"]);
        let files = walk_dir(dir.path(), &["src/gen.rs/"]).unwrap();
        assert_eq!(rel_paths(&files), ["gen.rs"]);
        let files = walk_dir(dir.path(), &["gen.rs"]).unwrap();
        assert_eq!(rel_paths(&files), ["src/gen.rs/inner.rs"]);
    }

    #[test]
    fn hidden_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "visible.rs");
        touch(dir.path(), ".git/config");
        touch(dir.path(), ".hidden/secret.py");

        let files = walk_dir(dir.path(), &[]).unwrap();
        assert_eq!(rel_paths(&files), ["visible.rs"]);
    }

    #[test]
    fn invalid_exclude_glob_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = walk(dir.path(), &["src/[broken".to_string()]);
        match result {
            Err(ArchguardError::Config(msg)) => assert!(msg.contains("src/[broken")),
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn missing_root_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = walk_dir(&dir.path().join("does-not-exist"), &[]);
        assert!(matches!(result, Err(ArchguardError::Io(_))));
    }
}
