//! File discovery (ARCHITECTURE.md Kap. 6, step 2): walking the source tree.
//!
//! [`walk`] traverses a root directory with the `ignore` crate — which
//! respects `.gitignore` files even outside a git checkout — keeps only
//! files whose extension maps to a known [`Lang`], drops everything matched
//! by the caller's `exclude` globs (compiled with `globset`) and returns the
//! result sorted by `rel_path` so downstream output is deterministic.

use std::path::{Path, PathBuf};

use archguard_core::ArchguardError;
use globset::{Glob, GlobSet, GlobSetBuilder};
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
    let excluded = build_excluder(excludes)?;
    let mut files = Vec::new();
    for entry in WalkBuilder::new(root).require_git(false).build() {
        let entry = entry.map_err(|e| ArchguardError::Io(std::io::Error::other(e.to_string())))?;
        // Depth 0 is `root` itself; only plain files can be sources (entries
        // with unknown file type, e.g. broken links, are skipped as well).
        if entry.depth() == 0 || !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(lang) = Lang::from_path(path) else {
            continue;
        };
        let rel_path = relative_path(root, path);
        if excluded.is_match(Path::new(&rel_path)) {
            continue;
        }
        files.push(SourceFile {
            path: path.to_path_buf(),
            rel_path,
            lang,
        });
    }
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(files)
}

/// Compile the `exclude` patterns into a single matcher.
fn build_excluder(excludes: &[String]) -> Result<GlobSet, ArchguardError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in excludes {
        let glob = Glob::new(pattern).map_err(|e| {
            ArchguardError::Config(format!("invalid exclude glob `{pattern}`: {e}"))
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| ArchguardError::Config(format!("could not build exclude matcher: {e}")))
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
        ] {
            assert_eq!(Lang::from_extension(ext), Some(lang), "ext {ext}");
        }
        assert_eq!(Lang::from_extension("txt"), None);
        assert_eq!(Lang::from_extension("scala"), None);
        assert_eq!(Lang::from_extension("kts"), None);
        assert_eq!(Lang::from_extension(""), None);
    }

    #[test]
    fn walk_detects_languages_and_skips_unknown_extensions() {
        let dir = tempfile::tempdir().unwrap();
        for rel in [
            "a.rs", "b.ts", "c.tsx", "d.js", "e.jsx", "f.mjs", "g.py", "h.java", "i.cs", "j.php",
            "k.kt",
        ] {
            touch(dir.path(), rel);
        }
        touch(dir.path(), "notes.txt");
        touch(dir.path(), "Other.scala");
        touch(dir.path(), "noext");

        let files = walk_dir(dir.path(), &[]).unwrap();
        assert_eq!(
            rel_paths(&files),
            [
                "a.rs", "b.ts", "c.tsx", "d.js", "e.jsx", "f.mjs", "g.py", "h.java", "i.cs",
                "j.php", "k.kt",
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
            ]
        );
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
