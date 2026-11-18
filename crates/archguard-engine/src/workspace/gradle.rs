//! Gradle Kotlin DSL subproject dependencies: `<configuration>(project(":a:b"))`
//! in a `build.gradle.kts`, checked against the literal `include(…)` calls of
//! the nearest `settings.gradle.kts`. Scripts are read, never run.
use std::path::Path;

use archguard_core::ServiceProject;
use archguard_extract::kotlin::{GradleScript, scan_gradle_script};

const MAX_SETTINGS: u64 = 1024 * 1024;

/// One `<configuration>(project(…))` of a build script.
pub(super) struct GradleDependency {
    pub(super) line: u32,
    /// The path as written, `(computed)` for a computed one.
    pub(super) written: String,
    /// The project id and workspace scope, or why there is none.
    pub(super) target: Result<(String, &'static str), &'static str>,
}

/// The project dependencies of the build script `source` in the directory
/// `dir`; `None` when its brackets are unbalanced.
pub(super) fn gradle_dependencies(
    root: &Path,
    dir: &str,
    source: &str,
    projects: &[ServiceProject],
) -> Option<Vec<GradleDependency>> {
    let script = scan_gradle_script(source)?;
    let mut settings = None;
    let mut out = Vec::new();
    for found in script.projects {
        let Some(configuration) = found.configuration else {
            continue;
        };
        let Some(path) = found.path else {
            out.push(GradleDependency {
                line: found.line,
                written: "(computed)".into(),
                target: Err("project path is computed, not a string literal"),
            });
            continue;
        };
        let settings = settings.get_or_insert_with(|| read_settings(root, dir));
        out.push(GradleDependency {
            line: found.line,
            target: target(&path, settings, projects).map(|id| (id, scope(&configuration))),
            written: path,
        });
    }
    Some(out)
}

/// The directory and calls of the nearest `settings.gradle.kts` at or above
/// `dir`.
fn read_settings(root: &Path, dir: &str) -> Result<(String, GradleScript), &'static str> {
    let mut dir = dir.to_string();
    loop {
        let at = |name: &str| {
            if dir == "." {
                name.to_string()
            } else {
                format!("{dir}/{name}")
            }
        };
        if root.join(at("settings.gradle")).is_file() {
            return Err("Groovy settings.gradle is not read");
        }
        if root.join(at("settings.gradle.kts")).is_file() {
            let bytes = crate::project_files::read_repo_file(
                root,
                Path::new(&at("settings.gradle.kts")),
                MAX_SETTINGS,
            )
            .map_err(|_| "settings.gradle.kts cannot be read safely")?;
            let source =
                String::from_utf8(bytes).map_err(|_| "settings.gradle.kts is not UTF-8")?;
            let script =
                scan_gradle_script(&source).ok_or("settings.gradle.kts has unbalanced brackets")?;
            return Ok((dir, script));
        }
        match dir.rsplit_once('/') {
            Some((parent, _)) => dir = parent.into(),
            None if dir != "." => dir = ".".into(),
            None => return Err("no settings.gradle.kts above the build script"),
        }
    }
}

/// The project id `path` (`:a:b`) names under the settings.
fn target(
    path: &str,
    settings: &Result<(String, GradleScript), &'static str>,
    projects: &[ServiceProject],
) -> Result<String, &'static str> {
    let (dir, script) = settings.as_ref().map_err(|reason| *reason)?;
    let name = path
        .strip_prefix(':')
        .ok_or("a relative project path is not resolved")?;
    let named = |written: &Option<String>| {
        written
            .as_ref()
            .map(|w| w.trim_start_matches(':').to_string())
    };
    // `include(":a:b")` also creates `:a`.
    let included = script
        .includes
        .iter()
        .filter_map(|i| named(&i.path))
        .any(|i| i == name || i.starts_with(&format!("{name}:")));
    if !included {
        return Err("project is not included by a literal include in settings.gradle.kts");
    }
    // `project(":a").projectDir = …` moves the directory.
    if script
        .projects
        .iter()
        .any(|p| named(&p.path).is_none_or(|p| p == name))
    {
        return Err("settings.gradle.kts may move the project directory");
    }
    if name
        .split(':')
        .any(|s| s.is_empty() || s == "." || s == ".." || s.contains(['/', '\\']))
    {
        return Err("project path contains unsupported syntax");
    }
    let sub = name.replace(':', "/");
    let dir = if dir == "." {
        sub
    } else {
        format!("{dir}/{sub}")
    };
    let mut candidates = projects
        .iter()
        .filter(|p| p.ecosystem == "gradle" && p.path == dir);
    match (candidates.next(), candidates.next()) {
        (Some(one), None) => Ok(one.id.clone()),
        _ => Err("included project has no single build script"),
    }
}

/// The workspace scope of a Gradle configuration.
fn scope(configuration: &str) -> &'static str {
    match configuration {
        c if c.starts_with("test") || c.contains("Test") => "test",
        "runtimeOnly" => "runtime",
        "compileOnly" => "provided",
        "annotationProcessor" | "kapt" | "ksp" => "build",
        _ => "compile",
    }
}
