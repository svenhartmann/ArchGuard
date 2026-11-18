//! C/C++ `#include` resolution (ADR 0017). A quoted include looks in the
//! including file's directory first, then in the project's include
//! directories; an angled include only in the include directories, and
//! is external (no warning) when none has it. Include directories come
//! from `compile_commands.json` (in the root or `build/`) and from literal
//! `include_directories` / `target_include_directories` arguments of the
//! `CMakeLists.txt` files above C/C++ sources. They are pooled for the
//! whole repository: a spec found in two of them stays unresolved.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

use archguard_core::NodeId;
use archguard_extract::{RawImport, TargetSpec, strip_root};

/// Include directories of one run, root-relative (`""` is the root).
#[derive(Debug, Default)]
pub(super) struct Includes {
    dirs: BTreeSet<String>,
    source_roots: Vec<String>,
}

impl Includes {
    /// Read the include directories for the C/C++ files at `rel_paths`;
    /// unreadable or malformed build files add nothing.
    pub(super) fn read<'a>(
        root: &Path,
        rel_paths: impl Iterator<Item = &'a str>,
        source_roots: &[String],
    ) -> Self {
        Includes {
            dirs: include_dirs(root, rel_paths),
            source_roots: source_roots.to_vec(),
        }
    }

    /// The file node an include of the file at `rel_path` (root-relative)
    /// resolves to; `None` for a gap or an external header.
    pub(super) fn resolve(
        &self,
        raw: &RawImport,
        rel_path: &str,
        node_of: &HashMap<String, NodeId>,
    ) -> Option<NodeId> {
        let node = |dir: &str, spec: &str| {
            let path = normalize(&format!("{dir}/{spec}"))?;
            node_of.get(&strip_root(&path, &self.source_roots)).copied()
        };
        let spec = match &raw.target {
            TargetSpec::Path(spec) => {
                let dir = rel_path.rsplit_once('/').map_or("", |(dir, _)| dir);
                if let Some(found) = node(dir, spec) {
                    return Some(found);
                }
                spec
            }
            TargetSpec::Symbol { path, .. } => path,
            TargetSpec::Qualified(_) => return None,
        };
        let found: BTreeSet<NodeId> = self.dirs.iter().filter_map(|d| node(d, spec)).collect();
        (found.len() == 1).then(|| *found.first().expect("one"))
    }
}

/// `a/./b/../c` → `a/c`; `None` when the path leaves the root or is absolute.
fn normalize(path: &str) -> Option<String> {
    if path.starts_with("//") {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Root-relative include directories from the build files of the C/C++
/// sources at `rel_paths`.
pub(crate) fn include_dirs<'a>(
    root: &Path,
    rel_paths: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut found = BTreeSet::new();
    for dir in [base.clone(), base.join("build")] {
        if let Ok(text) = fs::read_to_string(dir.join("compile_commands.json")) {
            found.extend(compile_commands(&text, &dir));
        }
    }
    let mut cmake_dirs = BTreeSet::from([String::new()]);
    for rel in rel_paths {
        let mut dir = rel;
        while let Some((parent, _)) = dir.rsplit_once('/') {
            cmake_dirs.insert(parent.to_string());
            dir = parent;
        }
    }
    for dir in cmake_dirs {
        if let Ok(text) = fs::read_to_string(root.join(&dir).join("CMakeLists.txt")) {
            found.extend(cmake(&text).into_iter().map(|d| base.join(&dir).join(d)));
        }
    }
    found
        .iter()
        .filter_map(|dir| relative(&base, dir))
        .collect()
}

/// `dir` as a root-relative `/`-joined path, `None` outside the root.
fn relative(base: &Path, dir: &Path) -> Option<String> {
    let dir = dir.canonicalize().ok()?;
    let rest = dir.strip_prefix(base).ok()?;
    let parts: Vec<String> = rest
        .components()
        .map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

/// `-I`, `-iquote`, `-isystem` and `/I` directories of every entry,
/// resolved against the entry's `directory` (a relative one against
/// `base`, the directory of the file).
fn compile_commands(text: &str, base: &Path) -> Vec<PathBuf> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in &entries {
        let directory = base.join(entry["directory"].as_str().unwrap_or(""));
        // shortcut: `command` is split at whitespace without shell quoting;
        // quoted include paths with spaces need `arguments`.
        let args: Vec<String> = match &entry["arguments"] {
            serde_json::Value::Array(args) => args
                .iter()
                .filter_map(|a| a.as_str().map(str::to_string))
                .collect(),
            _ => entry["command"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect(),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let dir = ["-I", "/I", "-iquote", "-isystem"]
                .iter()
                .find_map(|flag| arg.strip_prefix(flag))
                .map(|rest| match rest {
                    "" => args.next().map(String::as_str),
                    rest => Some(rest),
                });
            if let Some(Some(dir)) = dir {
                out.push(directory.join(dir));
            }
        }
    }
    out
}

/// Literal directories of `include_directories(…)` and
/// `target_include_directories(…)`, relative to the `CMakeLists.txt`;
/// `${CMAKE_CURRENT_SOURCE_DIR}` and `${CMAKE_CURRENT_LIST_DIR}` are that
/// directory, arguments with other variables or generator expressions are
/// skipped.
fn cmake(text: &str) -> Vec<String> {
    let code: String = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let lower = code.to_ascii_lowercase();
    let mut out = Vec::new();
    for (command, skip) in [
        ("target_include_directories", 1),
        ("include_directories", 0),
    ] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(command) {
            let start = from + at;
            from = start + command.len();
            // `include_directories` also matches inside the target command.
            let before = start.checked_sub(1).map(|i| lower.as_bytes()[i]);
            let whole = before.is_none_or(|b| !(b.is_ascii_alphanumeric() || b == b'_'));
            let Some(args) = code[from..]
                .trim_start()
                .strip_prefix('(')
                .filter(|_| whole)
            else {
                continue;
            };
            let args = &args[..args.find(')').unwrap_or(args.len())];
            out.extend(
                cmake_args(args)
                    .into_iter()
                    .skip(skip)
                    .filter_map(literal_dir),
            );
        }
    }
    out
}

/// Whitespace-separated arguments; double quotes group.
fn cmake_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in args.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    out.extend((!current.is_empty()).then_some(current));
    out
}

/// A directory argument without keywords, variables or generator expressions.
fn literal_dir(arg: String) -> Option<String> {
    let keyword = matches!(
        arg.to_ascii_uppercase().as_str(),
        "SYSTEM" | "BEFORE" | "AFTER" | "PUBLIC" | "PRIVATE" | "INTERFACE"
    );
    let arg = ["${CMAKE_CURRENT_SOURCE_DIR}", "${CMAKE_CURRENT_LIST_DIR}"]
        .iter()
        .find_map(|var| arg.strip_prefix(var))
        .map_or(arg.as_str(), |rest| rest.trim_start_matches('/'))
        .to_string();
    (!keyword && !arg.contains(['$', '<', '>'])).then_some(arg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_dots_and_refuses_to_leave_the_root() {
        assert_eq!(normalize("src/./a/../b.h").as_deref(), Some("src/b.h"));
        assert_eq!(normalize("/include/x.h").as_deref(), Some("include/x.h"));
        assert_eq!(normalize("../x.h"), None);
    }

    #[test]
    fn cmake_reads_literal_directories_only() {
        let text = "project(x)\n\
            include_directories(include \"third party\" ${GEN_DIR} ${CMAKE_CURRENT_SOURCE_DIR})\n\
            # include_directories(commented)\n\
            TARGET_INCLUDE_DIRECTORIES(app PUBLIC src/api PRIVATE $<BUILD_INTERFACE:gen> SYSTEM vendor)\n";
        assert_eq!(
            cmake(text),
            ["src/api", "vendor", "include", "third party", ""]
        );
    }

    #[test]
    fn compile_commands_read_every_include_flag() {
        let text = r#"[
            {"directory": "/repo/build", "command": "c++ -I../include -I /repo/lib -isystem/opt/x -DX=1 -c ../src/a.cpp"},
            {"directory": "sub", "arguments": ["cl", "/Isdk", "-iquote", "quoted"], "file": "b.c"}
        ]"#;
        let dirs: Vec<_> = compile_commands(text, Path::new("/repo"))
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            dirs,
            [
                "/repo/build/../include",
                "/repo/lib",
                "/opt/x",
                "/repo/sub/sdk",
                "/repo/sub/quoted"
            ]
        );
        assert!(compile_commands("not json", Path::new("/repo")).is_empty());
    }
}
