//! `/`-joined path helpers of the pipeline's resolution step.

/// `prefix/rest` with empty halves collapsed (the root namespace has an
/// empty prefix).
pub(super) fn join_module(prefix: &str, rest: &str) -> String {
    match (prefix.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => prefix.to_string(),
        (false, false) => format!("{prefix}/{rest}"),
    }
}

/// The directory part of a `/`-joined path; `""` for a path with no
/// directory component.
pub(super) fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// The path as `resolve_relative` returns it: the last component
/// without its file extension (a leading-dot file keeps its name).
pub(super) fn stem_path(path: &str) -> String {
    let (dir, last) = match path.rsplit_once('/') {
        Some((dir, last)) => (Some(dir), last),
        None => (None, path),
    };
    let stem = match last.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => last,
    };
    match dir {
        Some(dir) => format!("{dir}/{stem}"),
        None => stem.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_path_strips_only_the_last_extension() {
        assert_eq!(stem_path("a/b/util.ts"), "a/b/util");
        assert_eq!(stem_path("a/main.d.ts"), "a/main.d");
        assert_eq!(stem_path("script"), "script");
        assert_eq!(stem_path(".hidden"), ".hidden");
    }

    #[test]
    fn join_module_collapses_empty_halves() {
        assert_eq!(join_module("App", "Model"), "App/Model");
        assert_eq!(join_module("", "Model"), "Model");
        assert_eq!(join_module("App", ""), "App");
        assert_eq!(join_module("", ""), "");
    }
}
