//! The compiled `project.exclude` rule (L0), shared by every walk and every
//! crate that honours it. It lives here because the pattern semantics belong
//! to the configuration type in [`crate::settings`], not to one walker.

use crate::model::ArchguardError;
use globset::{Glob, GlobSet, GlobSetBuilder};

/// The `project.exclude` rule, shared by every walk that applies it.
///
/// Patterns are globs over `/`-joined root-relative paths, anchored at the
/// root (`vendor` is `vendor`, not `a/vendor`). A directory a pattern matches
/// is excluded with everything below it, as in `.gitignore`: `vendor`,
/// `vendor/`, `vendor/**` and `**/generated` all prune their directory. A
/// trailing `/` matches directories only, never a file of that name.
#[derive(Debug, Clone)]
pub struct Excludes {
    /// Patterns that match files and directories.
    any: GlobSet,
    /// Patterns that match directories only: `dir/` and the `dir` of `dir/**`.
    dirs: GlobSet,
}

impl Excludes {
    /// Compile `patterns`; an invalid glob is an [`ArchguardError::Config`].
    pub fn new(patterns: &[String]) -> Result<Self, ArchguardError> {
        let (mut any, mut dirs) = (GlobSetBuilder::new(), GlobSetBuilder::new());
        for pattern in patterns {
            let invalid = |e: globset::Error| {
                ArchguardError::Config(format!("invalid exclude glob `{pattern}`: {e}"))
            };
            if let Some(dir) = pattern.strip_suffix('/') {
                dirs.add(Glob::new(dir).map_err(invalid)?);
                continue;
            }
            any.add(Glob::new(pattern).map_err(invalid)?);
            // `dir/**` excludes the directory itself too, so the walk skips it
            // unentered; a prefix that does not compile alone (a trailing
            // escape) keeps the per-entry meaning.
            if let Some(Ok(dir)) = pattern.strip_suffix("/**").map(Glob::new) {
                dirs.add(dir);
            }
        }
        let build = |set: GlobSetBuilder| {
            set.build().map_err(|e| {
                ArchguardError::Config(format!("could not build exclude matcher: {e}"))
            })
        };
        Ok(Self {
            any: build(any)?,
            dirs: build(dirs)?,
        })
    }

    /// Whether the entry at root-relative `rel_path` is excluded. Only the
    /// entry itself is tested: a walk must prune a matching directory
    /// (e.g. `ignore::WalkBuilder::filter_entry`) so nothing below it is seen.
    pub fn is_match(&self, rel_path: &str, is_dir: bool) -> bool {
        self.any.is_match(rel_path) || (is_dir && self.dirs.is_match(rel_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excludes(patterns: &[&str]) -> Excludes {
        Excludes::new(
            &patterns
                .iter()
                .map(|p| (*p).to_string())
                .collect::<Vec<_>>(),
        )
        .expect("valid globs")
    }

    #[test]
    fn patterns_are_anchored_at_the_root() {
        let rule = excludes(&["vendor"]);
        assert!(rule.is_match("vendor", true));
        assert!(
            !rule.is_match("a/vendor", true),
            "not anchored below the root"
        );
    }

    #[test]
    fn a_trailing_slash_matches_directories_only() {
        let rule = excludes(&["generated/"]);
        assert!(rule.is_match("generated", true));
        assert!(
            !rule.is_match("generated", false),
            "a file of that name stays"
        );
    }

    #[test]
    fn a_double_star_pattern_also_prunes_its_own_directory() {
        let rule = excludes(&["vendor/**"]);
        assert!(
            rule.is_match("vendor", true),
            "the directory itself is pruned"
        );
        assert!(rule.is_match("vendor/lib.rs", false));
    }

    #[test]
    fn an_invalid_glob_is_a_config_error() {
        match Excludes::new(&["src/[broken".to_string()]) {
            Err(ArchguardError::Config(msg)) => assert!(msg.contains("src/[broken")),
            other => panic!("expected Config error, got {other:?}"),
        }
    }
}
