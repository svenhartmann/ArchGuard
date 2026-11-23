//! Whether a project's build file names Quarkus.
use std::path::Path;

use archguard_flow_common::extract_ctx::read_text;

/// The largest source or build file read, in bytes.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// The build files that make a directory a JVM project.
pub const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];

pub fn quarkus_build(root: &Path, dir: &str) -> bool {
    (BUILD_FILES.iter()).any(|b| {
        let rel = if dir.is_empty() {
            (*b).to_string()
        } else {
            format!("{dir}/{b}")
        };
        read_text(root, &rel, MAX_BYTES).is_ok_and(|t| t.contains("quarkus"))
    })
}
