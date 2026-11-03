//! Manifest project boundaries complement deployment declarations; no build command is run.
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path};

use archguard_core::{
    ArchguardError, Service, ServiceDiagnostic, ServiceEvidence, ServiceProject, ServiceTopology,
};
use roxmltree::{Document, Node};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_PROJECTS: usize = 2_000;
const MAX_ENTRIES: usize = 100_000;

fn diagnostic(report: &mut ServiceTopology, file: &str, message: &str) {
    report.complete = false;
    report.diagnostics.push(ServiceDiagnostic {
        file: file.into(),
        service: None,
        message: message.into(),
    });
}
fn manifest_kind(path: &Path) -> Option<&'static str> {
    match path.file_name()?.to_str()? {
        "pom.xml" => Some("maven"),
        "package.json" => Some("npm"),
        "Cargo.toml" => Some("cargo"),
        "go.mod" => Some("go"),
        "pyproject.toml" | "requirements.txt" | "setup.py" => Some("python"),
        "build.gradle" | "build.gradle.kts" => Some("gradle"),
        _ if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("csproj" | "fsproj" | "vbproj")
        ) =>
        {
            Some("dotnet")
        }
        _ => None,
    }
}
fn relative_name(path: &Path) -> Option<String> {
    path.components()
        .map(|component| match component {
            Component::Normal(part) => part
                .to_str()
                .filter(|part| !part.contains('\\'))
                .map(str::to_owned),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"))
}
pub(crate) fn read_manifest(root: &Path, relative: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, openat};
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let mut parent = openat(rustix::fs::CWD, root, flags, Mode::empty())?;
        for part in relative.parent().unwrap_or(Path::new("")).components() {
            parent = openat(&parent, Path::new(part.as_os_str()), flags, Mode::empty())?;
        }
        fs::File::from(openat(
            &parent,
            Path::new(
                relative
                    .file_name()
                    .ok_or(std::io::ErrorKind::InvalidInput)?,
            ),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?)
    };
    #[cfg(not(unix))]
    let file = {
        let mut path = root.to_path_buf();
        for component in relative.components() {
            path.push(component);
            if fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(std::io::ErrorKind::PermissionDenied.into());
            }
        }
        fs::File::open(path)?
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut source = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut source)?;
    if source.len() as u64 > MAX_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(source)
}

pub(crate) fn augment(
    root: &Path,
    discovery: &crate::services::Discovery,
    report: &mut ServiceTopology,
) -> Result<(), ArchguardError> {
    let walker = discovery.walk(root);
    let mut paths = Vec::new();
    for (index, entry) in walker.enumerate() {
        if index >= MAX_ENTRIES || paths.len() >= MAX_PROJECTS {
            diagnostic(
                report,
                ".",
                "Project discovery limit reached; remaining manifests were not analyzed.",
            );
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                diagnostic(
                    report,
                    ".",
                    "Project discovery could not read a directory; manifest coverage is incomplete.",
                );
                continue;
            }
        };
        if entry.depth() == 64 && entry.file_type().is_some_and(|kind| kind.is_dir()) {
            diagnostic(report, ".", "Project discovery depth limit reached.");
        }
        if entry.depth() == 0 {
            continue;
        }
        let Some(ecosystem) = manifest_kind(entry.path()) else {
            continue;
        };
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            ArchguardError::Analysis("project manifest escaped repository root".into())
        })?;
        let Some(name) = relative_name(relative) else {
            diagnostic(
                report,
                ".",
                "A manifest path has an unsupported encoding or ambiguous separator.",
            );
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            diagnostic(
                report,
                &name,
                "Non-regular or symlinked project manifest was not read.",
            );
            continue;
        }
        paths.push((name, ecosystem));
    }
    paths.sort();
    let mut total = 0usize;
    for (manifest, ecosystem) in paths {
        let source = match read_manifest(root, Path::new(&manifest)) {
            Ok(source) => source,
            Err(_) => {
                diagnostic(
                    report,
                    &manifest,
                    "Project manifest is unreadable, symlinked, too large or not valid UTF-8.",
                );
                continue;
            }
        };
        total += source.len();
        if total > 32 * 1024 * 1024 {
            diagnostic(
                report,
                &manifest,
                "Project manifest input limit reached (32 MiB).",
            );
            break;
        }
        let path = Path::new(&manifest)
            .parent()
            .and_then(Path::to_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(".")
            .to_owned();
        let fallback = Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project")
            .to_owned();
        let mut project = ServiceProject {
            id: format!("project:{manifest}"),
            name: fallback,
            path,
            manifest: manifest.clone(),
            ecosystem: ecosystem.into(),
            kind: "unknown".into(),
            runtime: None,
            evidence: Vec::new(),
        };
        let result = match ecosystem {
            "maven" | "dotnet" => parse_xml(&source, &mut project),
            "npm" => parse_npm(&source, &mut project),
            "cargo" => parse_cargo(&source, root, &mut project),
            "go" | "python" | "gradle" => {
                project.kind = "unknown".into();
                if ecosystem == "go" {
                    if let Some(name) = source
                        .lines()
                        .find_map(|l| l.trim().strip_prefix("module "))
                    {
                        project.name = name
                            .trim()
                            .trim_matches('"')
                            .rsplit('/')
                            .next()
                            .unwrap_or(&project.name)
                            .into();
                    }
                } else if ecosystem == "python"
                    && manifest.ends_with("pyproject.toml")
                    && let Ok(value) = source.parse::<toml::Value>()
                    && let Some(name) = value
                        .get("project")
                        .and_then(|p| p.get("name"))
                        .and_then(|v| v.as_str())
                {
                    project.name = name.into();
                }
                evidence(
                    &mut project,
                    "/",
                    None,
                    "Source project boundary from manifest; build scripts and dependency installation are not executed.",
                );
                Ok(())
            }
            _ => unreachable!(),
        };
        match result {
            Ok(()) => report.projects.push(project),
            Err(message) => diagnostic(report, &manifest, message),
        }
    }
    report.projects.sort_by(|a, b| a.id.cmp(&b.id));
    // Prefer the structured Python manifest to auxiliary requirements/setup files.
    let structured: BTreeSet<_> = report
        .projects
        .iter()
        .filter(|p| p.manifest.ends_with("pyproject.toml"))
        .map(|p| p.path.clone())
        .collect();
    report.projects.retain(|p| {
        !(p.ecosystem == "python"
            && structured.contains(&p.path)
            && !p.manifest.ends_with("pyproject.toml"))
    });
    let mut mapped = BTreeSet::new();
    let shared_contexts: BTreeSet<_> = report
        .services
        .iter()
        .filter_map(|s| s.build_context.as_ref())
        .filter(|context| {
            report
                .services
                .iter()
                .filter(|s| s.build_context.as_ref() == Some(context))
                .count()
                > 1
        })
        .cloned()
        .collect();
    for service in &mut report.services {
        let Some(context) = service.build_context.as_deref() else {
            continue;
        };
        let mut candidates: Vec<_> = report
            .projects
            .iter()
            .filter(|project| {
                !matches!(project.kind.as_str(), "workspace" | "tooling")
                    && !(shared_contexts.contains(context) && project.path == ".")
                    && (project.path == "."
                        || context == project.path
                        || context.starts_with(&(project.path.clone() + "/")))
            })
            .collect();
        candidates.sort_by_key(|project| std::cmp::Reverse(project.path.len()));
        if let Some(project) = candidates.first().filter(|first| {
            candidates
                .get(1)
                .is_none_or(|other| other.path.len() < first.path.len())
        }) {
            service.project = Some(project.id.clone());
            mapped.insert(project.id.clone());
        }
    }
    for project in &report.projects {
        // A Rust executable may be a CLI or batch job. Only deployment evidence
        // promotes it to a service; the application remains in the project inventory.
        if project.kind == "application"
            && project.ecosystem != "cargo"
            && !mapped.contains(&project.id)
        {
            report.services.push(Service {
                id: project.id.clone(),
                name: project.name.clone(),
                compose_file: String::new(),
                build_context: Some(project.path.clone()),
                networks: Vec::new(),
                profiles: Vec::new(),
                project: Some(project.id.clone()),
            });
        }
    }
    report.services.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(())
}
fn evidence(project: &mut ServiceProject, pointer: &str, line: Option<u32>, description: &str) {
    project.evidence.push(ServiceEvidence {
        file: project.manifest.clone(),
        pointer: pointer.into(),
        line,
        description: description.into(),
    });
}
fn literal(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 512
        && !value.contains("${")
        && !value.contains("$(")
        && !value.chars().any(char::is_control))
    .then_some(value)
}
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}
fn child_text<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|n| n.text()).and_then(literal)
}
fn parse_xml(source: &str, project: &mut ServiceProject) -> Result<(), &'static str> {
    let doc=Document::parse_with_options(source,roxmltree::ParsingOptions { allow_dtd:false,nodes_limit:50_000,..Default::default() }).map_err(|_|"Project XML could not be parsed within the supported limits; DTD/external entities are not resolved.")?;
    let root = doc.root_element();
    if project.ecosystem == "maven" {
        if root.tag_name().name() != "project" {
            return Err("Maven manifest must have a project root element.");
        }
        let name=child_text(root,"artifactId").ok_or("Maven project has no literal direct artifactId; inherited/interpolated project identity is unresolved.")?;
        project.name = name.into();
        let packaging = child_text(root, "packaging").unwrap_or("jar");
        project.kind = if packaging == "pom" {
            "workspace"
        } else {
            "library"
        }
        .into();
        evidence(
            project,
            "/project/artifactId",
            child(root, "artifactId").map(|n| doc.text_pos_at(n.range().start).row),
            "Maven project identity is declared directly in this manifest.",
        );
        // An aggregator may configure plugins inherited by children; it does
        // not itself produce a deployable artifact.
        if packaging == "pom" {
            return Ok(());
        }
        if packaging == "war" {
            project.kind = "application".into();
            project.runtime = Some("java-web".into());
            evidence(
                project,
                "/project/packaging",
                None,
                "WAR packaging declares a deployable Java web application.",
            );
        }
        if let Some(plugins) = child(root, "build").and_then(|build| child(build, "plugins")) {
            for plugin in plugins
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "plugin")
            {
                let artifact = child_text(plugin, "artifactId").unwrap_or("");
                let group = child_text(plugin, "groupId").unwrap_or("");
                let runtime = match (group, artifact) {
                    ("com.microsoft.azure", "azure-functions-maven-plugin") => "azure-functions",
                    ("org.springframework.boot", "spring-boot-maven-plugin") => "spring-boot",
                    _ => continue,
                };
                project.kind = "application".into();
                project.runtime = Some(runtime.into());
                evidence(
                    project,
                    "/project/build/plugins/plugin",
                    Some(doc.text_pos_at(plugin.range().start).row),
                    &format!(
                        "Active build plugin {group}:{artifact} declares an application build boundary; profiles and pluginManagement are not activated."
                    ),
                );
                break;
            }
        }
    } else {
        if root.tag_name().name() != "Project" {
            return Err(".NET manifest must have a Project root element.");
        }
        project.name = Path::new(&project.manifest)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&project.name)
            .into();
        project.kind = "library".into();
        evidence(
            project,
            "/Project",
            Some(doc.text_pos_at(root.range().start).row),
            ".NET project boundary is declared by this manifest; imported MSBuild properties are not evaluated.",
        );
        let property = |name| {
            root.children()
                .filter(|n| {
                    n.is_element()
                        && n.tag_name().name() == "PropertyGroup"
                        && n.attribute("Condition").is_none()
                })
                .flat_map(|n| n.children())
                .rfind(|n| {
                    n.is_element()
                        && n.tag_name().name() == name
                        && n.attribute("Condition").is_none()
                })
                .and_then(|n| n.text())
                .and_then(literal)
        };
        let test_reference = root
            .children()
            .filter(|n| {
                n.is_element()
                    && n.tag_name().name() == "ItemGroup"
                    && n.attribute("Condition").is_none()
            })
            .flat_map(|n| n.children())
            .any(|n| {
                n.is_element()
                    && n.tag_name().name() == "PackageReference"
                    && matches!(
                        n.attribute("Include"),
                        Some("Microsoft.NET.Test.Sdk" | "SharpFuzz")
                    )
                    && n.attribute("Condition").is_none()
            });
        if let Some(name) = property("AssemblyName") {
            project.name = name.into();
        }
        if property("IsTestProject").is_some_and(|p| p.eq_ignore_ascii_case("true"))
            || test_reference
        {
            project.kind = "tooling".into();
            evidence(
                project,
                "/Project",
                None,
                "Explicit test/fuzz-project metadata identifies tooling, not a software service.",
            );
        } else if root
            .attribute("Sdk")
            .is_some_and(|sdk| sdk.split(';').any(|s| s == "Microsoft.NET.Sdk.Web"))
            || matches!(property("OutputType"), Some("Exe" | "WinExe"))
        {
            project.kind = "application".into();
            project.runtime = Some("dotnet".into());
            evidence(
                project,
                "/Project",
                None,
                "Web SDK or executable OutputType declares an application project.",
            );
        }
    }
    Ok(())
}

fn start_runtime(script: &str) -> Option<&'static str> {
    if script.contains([';', '|', '&', '\n']) {
        return None;
    }
    let words: Vec<_> = script.split_whitespace().collect();
    let mut start = usize::from(words.first() == Some(&"cross-env"));
    while words.get(start).is_some_and(|word| {
        word.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    }) {
        start += 1;
    }
    if words.get(start) == Some(&"npx") {
        start += 1;
    }
    let words = &words[start..];
    match words {
        ["node", rest @ ..]
            if rest.iter().any(|word| !word.starts_with('-'))
                && !rest.iter().any(|word| word.starts_with("--test")) =>
        {
            Some("node")
        }
        ["bun", "run", entry, ..]
            if [".js", ".ts", ".jsx", ".tsx", ".mjs", ".cjs"]
                .iter()
                .any(|extension| entry.ends_with(extension)) =>
        {
            Some("bun")
        }
        ["next", "start" | "dev", ..] => Some("nextjs"),
        ["nuxt", "start" | "dev", ..] => Some("nuxt"),
        ["vite", rest @ ..]
            if !rest.contains(&"--help")
                && !rest.contains(&"--version")
                && rest.first().is_none_or(|word| {
                    word.starts_with('-') || matches!(*word, "dev" | "serve" | "preview")
                }) =>
        {
            Some("vite")
        }
        ["ng", "serve", ..] => Some("angular"),
        ["react-scripts", "start", ..] => Some("react"),
        ["nest", "start", ..] => Some("nestjs"),
        _ => None,
    }
}
fn parse_npm(source: &str, project: &mut ServiceProject) -> Result<(), &'static str> {
    let doc: serde_json::Value = serde_json::from_str(source).map_err(
        |_| "package.json is not valid JSON; project identity/classification is unavailable.",
    )?;
    if !doc.is_object() {
        return Err("package.json must be a JSON object.");
    }
    if let Some(name) = doc.get("name").and_then(|v| v.as_str()).and_then(literal) {
        project.name = name.into();
    }
    evidence(
        project,
        "/name",
        None,
        "Project boundary is declared by package.json; scripts are inspected as metadata and never executed.",
    );
    let scripts = doc.get("scripts").and_then(|v| v.as_object());
    for key in ["start", "dev", "serve"] {
        if let Some(runtime) = scripts
            .and_then(|scripts| scripts.get(key))
            .and_then(|v| v.as_str())
            .and_then(start_runtime)
        {
            project.kind = "application".into();
            project.runtime = Some(runtime.into());
            evidence(
                project,
                &format!("/scripts/{key}"),
                None,
                "A declared script directly names a supported application runtime; command contents are not exported or executed.",
            );
            return Ok(());
        }
    }
    if ["main", "module", "exports", "types", "typings"]
        .iter()
        .any(|key| doc.get(key).is_some())
    {
        project.kind = "library".into();
        evidence(
            project,
            "/",
            None,
            "Published module entry metadata identifies a library project.",
        );
    } else if doc.get("workspaces").is_some() {
        project.kind = "workspace".into();
        evidence(
            project,
            "/workspaces",
            None,
            "Workspace metadata groups child projects; membership is discovered from local manifests.",
        );
    } else if scripts.is_some_and(|scripts| scripts.contains_key("test"))
        && doc
            .get("dependencies")
            .and_then(|v| v.as_object())
            .is_none_or(|deps| deps.is_empty())
        || doc
            .get("devDependencies")
            .and_then(|v| v.as_object())
            .is_some_and(|deps| {
                ["@playwright/test", "bats", "cypress"]
                    .iter()
                    .any(|name| deps.contains_key(*name))
            })
    {
        project.kind = "tooling".into();
        evidence(
            project,
            "/",
            None,
            "Test metadata without a supported application entry identifies tooling.",
        );
    }
    Ok(())
}
fn parse_cargo(
    source: &str,
    root: &Path,
    project: &mut ServiceProject,
) -> Result<(), &'static str> {
    let doc: toml::Value = toml::from_str(source).map_err(
        |_| "Cargo.toml is not valid TOML; project identity/classification is unavailable.",
    )?;
    if let Some(package) = doc.get("package") {
        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .and_then(literal)
            .ok_or("Cargo package identity is not a literal name.")?;
        project.name = name.into();
        project.kind = "library".into();
        evidence(
            project,
            "/package/name",
            None,
            "Cargo package boundary is declared by this manifest.",
        );
        let main = Path::new(&project.path).join("src/main.rs");
        let main = if project.path == "." {
            Path::new("src/main.rs").to_path_buf()
        } else {
            main
        };
        let default_main = package.get("autobins").and_then(|v| v.as_bool()) != Some(false)
            && read_manifest(root, &main).is_ok();
        if doc
            .get("bin")
            .and_then(|v| v.as_array())
            .is_some_and(|bins| !bins.is_empty())
            || default_main
        {
            project.kind = "application".into();
            project.runtime = Some("rust".into());
            evidence(
                project,
                "/bin",
                None,
                "An explicit binary target or local src/main.rs declares an executable Rust application.",
            );
        }
    } else if doc.get("workspace").is_some() {
        project.kind = "workspace".into();
        evidence(
            project,
            "/workspace",
            None,
            "Cargo workspace metadata groups child manifest projects.",
        );
    } else {
        return Err("Cargo.toml declares neither a package nor a workspace.");
    }
    Ok(())
}
