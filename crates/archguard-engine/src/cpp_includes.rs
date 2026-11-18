//! C/C++ `#include` resolution (ADR 0017). A quoted include looks in the
//! including file's directory first, then in the project's include
//! directories; an angled include only in the include directories, and
//! is external (no warning) when none has it. Include directories come
//! from `compile_commands.json` (in the root or `build/`) and from literal
//! `include_directories` / `target_include_directories` arguments of the
//! `CMakeLists.txt` files above C/C++ sources. They are pooled for the
//! whole repository: a spec found in two of them stays unresolved.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use archguard_core::NodeId;
use archguard_extract::{Lang, RawImport, SourceFile, TargetSpec, strip_root};

use crate::project_files::{read_manifest, read_repo_file};

/// Include directories of one run, root-relative (`""` is the root).
#[derive(Debug, Default)]
pub(super) struct Includes {
    dirs: BTreeSet<String>,
    source_roots: Vec<String>,
}

impl Includes {
    /// Resolution over `dirs` from [`include_dirs`].
    pub(super) fn new(dirs: BTreeSet<String>, source_roots: &[String]) -> Self {
        Includes {
            dirs,
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

/// Include directories of the C/C++ files among `files`; empty, and no
/// build file read, without any.
pub(crate) fn cpp_include_dirs(root: &Path, files: &[SourceFile]) -> BTreeSet<String> {
    let mut cpp = files
        .iter()
        .filter(|file| file.lang == Lang::Cpp)
        .peekable();
    match cpp.peek() {
        Some(_) => include_dirs(root, cpp.map(|file| file.rel_path.as_str())),
        None => BTreeSet::new(),
    }
}

/// Upper bound of a `compile_commands.json`; large builds list every file.
const COMPILE_COMMANDS_LIMIT: u64 = 64 * 1024 * 1024;

/// Root-relative include directories from the build files of the C/C++
/// sources at `rel_paths`. Build files are read without following symlinks
/// and only as bounded regular files; unreadable or malformed ones add nothing.
fn include_dirs<'a>(root: &Path, rel_paths: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut found = BTreeSet::new();
    for dir in ["", "build"] {
        let path = Path::new(dir).join("compile_commands.json");
        if let Ok(bytes) = read_repo_file(&base, &path, COMPILE_COMMANDS_LIMIT) {
            found.extend(compile_commands(
                &String::from_utf8_lossy(&bytes),
                &base.join(dir),
            ));
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
        if let Ok(text) = read_manifest(&base, &Path::new(&dir).join("CMakeLists.txt")) {
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
    use std::fs;

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

    #[cfg(unix)]
    #[test]
    fn build_files_behind_symlinks_or_fifos_are_not_read() {
        let (root, outside) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root_dir = root.path().canonicalize().unwrap();
        fs::create_dir_all(root_dir.join("inc")).unwrap();
        fs::create_dir_all(root_dir.join("src")).unwrap();
        let commands = format!(
            r#"[{{"directory": "{}", "arguments": ["-Iinc"]}}]"#,
            root_dir.display()
        );
        fs::write(outside.path().join("compile_commands.json"), commands).unwrap();
        fs::write(
            outside.path().join("CMakeLists.txt"),
            "include_directories(../inc)",
        )
        .unwrap();
        let link = std::os::unix::fs::symlink;
        link(
            outside.path().join("compile_commands.json"),
            root_dir.join("compile_commands.json"),
        )
        .unwrap();
        link(
            outside.path().join("CMakeLists.txt"),
            root_dir.join("src/CMakeLists.txt"),
        )
        .unwrap();
        fs::create_dir_all(root_dir.join("build")).unwrap();
        let fifo = std::process::Command::new("mkfifo")
            .arg(root_dir.join("build/compile_commands.json"))
            .status()
            .unwrap();
        assert!(fifo.success());
        let (send, receive) = std::sync::mpsc::channel();
        let scanned = root_dir.clone();
        std::thread::spawn(move || {
            send.send(include_dirs(&scanned, ["src/a.cpp"].into_iter()))
                .unwrap();
        });
        let dirs = receive
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a FIFO build file must not block the scan");
        assert!(dirs.is_empty(), "{dirs:?}");
    }
}
