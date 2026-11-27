//! Whether a project's build file names Quarkus.
use std::path::Path;

use archguard_flow_common::extract_ctx::read_text;

/// The largest source or build file read, in bytes.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// The build files that make a directory a JVM project.
pub const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];

/// The texts of the build files of `dir` that exist. A missing file is normal (a
/// project has one of the three); one that exists but cannot be read goes to
/// `unread(file, message)`, because what it says is unknown.
pub fn read_builds(root: &Path, dir: &str, unread: &mut dyn FnMut(&str, String)) -> Vec<String> {
    let mut texts = Vec::new();
    for b in BUILD_FILES {
        let rel = if dir.is_empty() {
            b.to_string()
        } else {
            format!("{dir}/{b}")
        };
        match read_text(root, &rel, MAX_BYTES) {
            Ok(text) => texts.push(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => unread(&rel, format!("Build file not read: {e}")),
        }
    }
    texts
}

pub fn quarkus_build(root: &Path, dir: &str, unread: &mut dyn FnMut(&str, String)) -> bool {
    (read_builds(root, dir, unread).iter()).any(|t| t.contains("quarkus"))
}
