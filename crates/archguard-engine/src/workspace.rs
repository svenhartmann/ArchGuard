//! Explicit local build-project dependencies. Never runs build tools or fetches packages.
use archguard_core::{
    ArchguardError, GraphBuilder, Ref, ServiceEvidence, ServiceProject, ServiceTopology,
    WorkspaceDependency, WorkspaceReport, WorkspaceUnresolved,
};
use roxmltree::{Document, Node};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_DECLARATIONS: usize = 20_000;

pub fn analyze_workspace(root: &Path) -> Result<WorkspaceReport, ArchguardError> {
    let topology = crate::services::analyze_services(root)?;
    analyze_workspace_with_topology(root, &topology)
}

pub fn analyze_workspace_with_topology(
    root: &Path,
    topology: &ServiceTopology,
) -> Result<WorkspaceReport, ArchguardError> {
    let root = root.canonicalize()?;
    let mut report = WorkspaceReport {
        schema_version:1, root:root.to_string_lossy().into_owned(), status:"ready".into(),
        projects:topology.projects.clone(), dependencies:vec![],cycles:vec![],unresolved:vec![],
        diagnostics:topology.diagnostics.iter().map(|d|format!("{}: {}",d.file,d.message)).collect(),
        limitations:vec![
            "Edges describe explicit local manifest dependencies, not observed calls, successful builds or deployed requests. Aggregator membership and equal names alone create no edge.".into(),
            "Maven uses statically resolved coordinates, properties and local parents; profiles, imported BOMs, settings, relocation and external parent models are not evaluated. Registry-only npm/Cargo references are not assumed local.".into(),
            "npm workspace links require package.json workspace membership; pnpm-only workspace configuration and complex workspace version ranges are unresolved. Cargo features/platform conditions and MSBuild conditions are not executed; optional, test and build declarations retain their scopes.".into(),
            "Cycles include all resolved declared scopes, including optional/test/build edges, and are SCC groups rather than observed runtime paths. Path dependencies do not prove build-tool version compatibility.".into(),
        ],
    };
    if !topology.complete {
        report.status = "partial".into();
    }
    let mut sources = BTreeMap::new();
    let mut total = 0usize;
    for project in &topology.projects {
        match crate::project_files::read_repo_file(
            &root,
            Path::new(&project.manifest),
            MAX_MANIFEST,
        ) {
            Ok(bytes) => {
                total += bytes.len();
                if total > 32 * 1024 * 1024 {
                    report
                        .diagnostics
                        .push("Workspace manifest input limit reached (32 MiB).".into());
                    report.status = "partial".into();
                    break;
                }
                match String::from_utf8(bytes) {
                    Ok(s) => {
                        sources.insert(project.id.clone(), s);
                    }
                    Err(_) => {
                        report
                            .diagnostics
                            .push(format!("{}: manifest is not UTF-8", project.manifest));
                        report.status = "partial".into();
                    }
                }
            }
            Err(_) => {
                report.diagnostics.push(format!(
                    "{}: manifest cannot be read safely",
                    project.manifest
                ));
                report.status = "partial".into();
            }
        }
    }
    let mut maven = BTreeMap::new();
    let mut npm = BTreeMap::new();
    let mut cargo = BTreeMap::new();
    for project in &topology.projects {
        let Some(source) = sources.get(&project.id) else {
            continue;
        };
        let result = match project.ecosystem.as_str() {
            "maven" => parse_maven(project, source).map(|m| {
                maven.insert(project.id.clone(), m);
            }),
            "npm" => serde_json::from_str::<Value>(source)
                .map(|v| {
                    npm.insert(project.id.clone(), v);
                })
                .map_err(|_| "Invalid npm JSON".into()),
            "cargo" => toml::from_str::<toml::Value>(source)
                .map(|v| {
                    cargo.insert(project.id.clone(), v);
                })
                .map_err(|_| "Invalid Cargo TOML".into()),
            "dotnet" => Ok(()),
            _ => Err("Unsupported manifest ecosystem".into()),
        };
        if let Err(message) = result {
            report
                .diagnostics
                .push(format!("{}: {message}", project.manifest));
            report.status = "partial".into();
        }
    }
    let mut effective = BTreeMap::new();
    for id in maven.keys() {
        effective_maven(
            id,
            &maven,
            &mut effective,
            &mut BTreeSet::new(),
            &mut report,
            0,
        );
    }
    for project in &topology.projects {
        if report.dependencies.len() + report.unresolved.len() >= MAX_DECLARATIONS {
            report.status = "partial".into();
            report.diagnostics.push(
                "Workspace declaration limit reached; additional dependencies were not analyzed."
                    .into(),
            );
            break;
        }
        match project.ecosystem.as_str() {
            "maven" => resolve_maven(project, &maven, &effective, &mut report),
            "npm" => resolve_npm(&root, project, &npm, &mut report),
            "cargo" => resolve_cargo(&root, project, &cargo, &mut report),
            "dotnet" => {
                if let Some(source) = sources.get(&project.id) {
                    resolve_dotnet(&root, project, source, &mut report);
                }
            }
            _ => {}
        }
    }
    report.dependencies.sort_by(|a, b| {
        (&a.from, &a.to, &a.scope, &a.kind).cmp(&(&b.from, &b.to, &b.scope, &b.kind))
    });
    report.unresolved.sort_by(|a, b| {
        (&a.project, &a.target, &a.reason).cmp(&(&b.project, &b.target, &b.reason))
    });
    report.unresolved.dedup();
    let mut graph = GraphBuilder::new();
    let mut ids = BTreeMap::new();
    for project in &report.projects {
        ids.insert(project.id.clone(), graph.ensure_path(&project.id, None));
    }
    for dep in &report.dependencies {
        graph.edge(
            ids[&dep.from],
            ids[&dep.to],
            Ref {
                file: dep.evidence[0].file.clone(),
                line: dep.evidence[0].line.unwrap_or(1),
                raw: dep.evidence[0].description.clone(),
            },
        )?;
    }
    let graph = graph.build(report.root.clone());
    report.cycles = archguard_analysis::find_sccs(&graph)
        .into_iter()
        .map(|scc| {
            scc.node_ids
                .into_iter()
                .map(|id| graph.nodes[id.0 as usize].name.clone())
                .collect()
        })
        .collect();
    if !report.unresolved.is_empty() {
        report.status = "partial".into();
    }
    if report.projects.is_empty() && report.status == "ready" {
        report.status = "not_applicable".into();
    }
    report.diagnostics.sort();
    report.diagnostics.dedup();
    Ok(report)
}

fn evidence(
    project: &ServiceProject,
    pointer: &str,
    line: Option<u32>,
    description: &str,
) -> ServiceEvidence {
    ServiceEvidence {
        file: project.manifest.clone(),
        pointer: pointer.into(),
        line,
        description: description.into(),
    }
}
fn unresolved(
    report: &mut WorkspaceReport,
    project: &ServiceProject,
    target: &str,
    reason: &str,
    evidence: ServiceEvidence,
) {
    report.status = "partial".into();
    if report.unresolved.len() + report.dependencies.len() >= MAX_DECLARATIONS {
        if !report
            .diagnostics
            .iter()
            .any(|s| s.starts_with("Workspace declaration limit"))
        {
            report.diagnostics.push(
                "Workspace declaration limit reached; additional dependencies were not analyzed."
                    .into(),
            );
        }
        return;
    }
    report.unresolved.push(WorkspaceUnresolved {
        project: project.id.clone(),
        target: target.chars().take(1024).collect(),
        reason: reason.into(),
        evidence: vec![evidence],
    });
}
fn dependency(
    report: &mut WorkspaceReport,
    project: &ServiceProject,
    target: &str,
    scope: &str,
    kind: &str,
    evidence: ServiceEvidence,
) {
    if target == project.id {
        unresolved(
            report,
            project,
            target,
            "self dependency does not identify a separate local project",
            evidence,
        );
        return;
    }
    if report.dependencies.len() + report.unresolved.len() >= MAX_DECLARATIONS {
        unresolved(report, project, target, "declaration limit", evidence);
        return;
    }
    if let Some(existing) = report
        .dependencies
        .iter_mut()
        .find(|d| d.from == project.id && d.to == target && d.scope == scope && d.kind == kind)
    {
        if !existing.evidence.contains(&evidence) {
            existing.evidence.push(evidence);
        }
    } else {
        report.dependencies.push(WorkspaceDependency {
            from: project.id.clone(),
            to: target.into(),
            scope: scope.into(),
            kind: kind.into(),
            evidence: vec![evidence],
        });
    }
}
fn literal(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 1024
        && !s.contains("${")
        && !s.contains("$(")
        && !s.chars().any(char::is_control)
}
fn local_path(base: &str, value: &str) -> Option<String> {
    if !literal(value) || value.contains(['*', '?', '{', '}']) || value.contains(':') {
        return None;
    }
    let value = value.replace('\\', "/");
    let path = Path::new(&value);
    if path.is_absolute() {
        return None;
    }
    let mut parts = if base == "." {
        Vec::new()
    } else {
        base.split('/').map(str::to_owned).collect()
    };
    for c in path.components() {
        match c {
            Component::Normal(n) => parts.push(n.to_str()?.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            _ => return None,
        }
    }
    Some(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}
fn path_target(
    root: &Path,
    project: &ServiceProject,
    value: &str,
    manifest: Option<&str>,
    ecosystem: &str,
    report: &WorkspaceReport,
) -> Result<String, &'static str> {
    let mut path = local_path(&project.path, value).ok_or(
        "path is dynamic, absolute, contains unsupported syntax or escapes the repository",
    )?;
    if let Some(name) = manifest {
        path = if path == "." {
            name.into()
        } else {
            format!("{path}/{name}")
        };
    }
    crate::project_files::read_repo_file(root, Path::new(&path), MAX_MANIFEST)
        .map_err(|_| "local manifest is missing, symlinked, outside scope or unreadable")?;
    let candidates: Vec<_> = report
        .projects
        .iter()
        .filter(|p| p.manifest == path && p.ecosystem == ecosystem)
        .collect();
    if candidates.len() != 1 {
        return Err("path does not identify exactly one discovered local project");
    }
    Ok(candidates[0].id.clone())
}
fn xml(source: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 50_000,
            ..Default::default()
        },
    )
    .map_err(|_| "XML is invalid or exceeds safe parsing limits".into())
}
fn child<'a, 'b>(node: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}
fn text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[derive(Clone)]
struct MavenDependency {
    group: String,
    artifact: String,
    version: Option<String>,
    scope: Option<String>,
    optional: Option<String>,
    unsupported: bool,
    evidence: ServiceEvidence,
}
#[derive(Clone)]
struct Maven {
    project: ServiceProject,
    group: Option<String>,
    artifact: String,
    version: Option<String>,
    parent: Option<(String, String, String, Option<String>)>,
    properties: BTreeMap<String, String>,
    dependencies: Vec<MavenDependency>,
    management: Vec<MavenDependency>,
    conditional: Vec<MavenDependency>,
}
#[derive(Clone)]
struct EffectiveMaven {
    group: String,
    artifact: String,
    version: String,
    properties: BTreeMap<String, String>,
    dependencies: Vec<MavenDependency>,
    management: Vec<MavenDependency>,
}
fn parse_maven(project: &ServiceProject, source: &str) -> Result<Maven, String> {
    let doc = xml(source)?;
    let node = doc.root_element();
    let properties = child(node, "properties")
        .map(|p| {
            p.children()
                .filter(|n| n.is_element())
                .filter_map(|n| {
                    n.text()
                        .map(|s| (n.tag_name().name().to_owned(), s.trim().to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let parse_deps = |n: Option<Node<'_, '_>>, pointer: &str| -> Vec<MavenDependency> {
        n.map(|deps| {
            deps.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "dependency")
                .take(MAX_DECLARATIONS + 1)
                .map(|d| MavenDependency {
                    group: text(d, "groupId").unwrap_or_default(),
                    artifact: text(d, "artifactId").unwrap_or_default(),
                    version: text(d, "version"),
                    scope: text(d, "scope"),
                    optional: text(d, "optional"),
                    unsupported: text(d, "classifier").is_some()
                        || text(d, "type").is_some_and(|v| v != "jar"),
                    evidence: evidence(
                        project,
                        pointer,
                        Some(doc.text_pos_at(d.range().start).row),
                        "Explicit Maven dependency declaration.",
                    ),
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let dependencies = parse_deps(
        child(node, "dependencies"),
        "/project/dependencies/dependency",
    );
    let management = parse_deps(
        child(node, "dependencyManagement").and_then(|n| child(n, "dependencies")),
        "/project/dependencyManagement/dependencies/dependency",
    );
    let conditional = child(node, "profiles")
        .map(|profiles| {
            profiles
                .children()
                .filter(|n| n.is_element())
                .flat_map(|profile| {
                    parse_deps(
                        child(profile, "dependencies"),
                        "/project/profiles/profile/dependencies/dependency",
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let parent = child(node, "parent").map(|p| {
        (
            text(p, "groupId").unwrap_or_default(),
            text(p, "artifactId").unwrap_or_default(),
            text(p, "version").unwrap_or_default(),
            match child(p, "relativePath") {
                Some(n) => n
                    .text()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                None => Some("../pom.xml".into()),
            },
        )
    });
    Ok(Maven {
        project: project.clone(),
        group: text(node, "groupId"),
        artifact: text(node, "artifactId").unwrap_or_default(),
        version: text(node, "version"),
        parent,
        properties,
        dependencies,
        management,
        conditional,
    })
}
fn interpolate(value: &str, properties: &BTreeMap<String, String>) -> Option<String> {
    let mut value = value.to_owned();
    for _ in 0..32 {
        let Some(start) = value.find("${") else {
            return literal(&value).then_some(value);
        };
        let rest = &value[start + 2..];
        let end = rest.find('}')?;
        let replacement = properties.get(&rest[..end])?;
        let next = format!("{}{}{}", &value[..start], replacement, &rest[end + 1..]);
        if next == value || next.len() > 1024 {
            return None;
        }
        value = next;
    }
    None
}
fn effective_maven(
    id: &str,
    all: &BTreeMap<String, Maven>,
    cache: &mut BTreeMap<String, EffectiveMaven>,
    visiting: &mut BTreeSet<String>,
    report: &mut WorkspaceReport,
    depth: usize,
) -> Option<EffectiveMaven> {
    if let Some(v) = cache.get(id) {
        return Some(v.clone());
    }
    let m = all.get(id)?;
    if depth >= 32 || !visiting.insert(id.into()) {
        unresolved(
            report,
            &m.project,
            "parent",
            "cyclic or over-depth local Maven parent",
            evidence(
                &m.project,
                "/project/parent",
                None,
                "Local parent inheritance.",
            ),
        );
        return None;
    }
    let parent = if let Some((g, a, v, relative)) = &m.parent {
        let candidate = relative
            .as_ref()
            .and_then(|path| local_path(&m.project.path, path))
            .and_then(|path| all.values().find(|other| other.project.manifest == path));
        if let Some(candidate) = candidate {
            let resolved = effective_maven(
                &candidate.project.id,
                all,
                cache,
                visiting,
                report,
                depth + 1,
            );
            resolved.filter(|p| {
                let mut props = p.properties.clone();
                props.extend(m.properties.clone());
                interpolate(g, &props).as_deref() == Some(&p.group)
                    && interpolate(a, &props).as_deref() == Some(&p.artifact)
                    && interpolate(v, &props).as_deref() == Some(&p.version)
            })
        } else {
            None
        }
    } else {
        None
    };
    if m.parent.is_some() && parent.is_none() {
        unresolved(
            report,
            &m.project,
            "parent",
            "external, mismatched or unresolved parent model may contribute dependencies and is not evaluated",
            evidence(
                &m.project,
                "/project/parent",
                None,
                "Only a matching local parent model can supply inherited dependency declarations.",
            ),
        );
    }
    let mut props = parent
        .as_ref()
        .map(|p| p.properties.clone())
        .unwrap_or_default();
    props.extend(m.properties.clone());
    if let Some((g, a, v, _)) = &m.parent {
        for (key, value) in [("groupId", g), ("artifactId", a), ("version", v)] {
            props.insert(format!("project.parent.{key}"), value.clone());
            props.insert(format!("parent.{key}"), value.clone());
        }
    }
    let raw_group = m
        .group
        .clone()
        .or_else(|| parent.as_ref().map(|p| p.group.clone()))
        .or_else(|| m.parent.as_ref().map(|p| p.0.clone()))
        .unwrap_or_default();
    let raw_version = m
        .version
        .clone()
        .or_else(|| parent.as_ref().map(|p| p.version.clone()))
        .or_else(|| m.parent.as_ref().map(|p| p.2.clone()))
        .unwrap_or_default();
    for (key, value) in [
        ("groupId", &raw_group),
        ("artifactId", &m.artifact),
        ("version", &raw_version),
    ] {
        props.insert(format!("project.{key}"), value.clone());
        props.insert(format!("pom.{key}"), value.clone());
    }
    let coordinates = (
        interpolate(&raw_group, &props),
        interpolate(&m.artifact, &props),
        interpolate(&raw_version, &props),
    );
    visiting.remove(id);
    let (Some(group), Some(artifact), Some(version)) = coordinates else {
        unresolved(
            report,
            &m.project,
            &m.artifact,
            "Maven project coordinates contain unresolved properties or missing values",
            evidence(
                &m.project,
                "/project",
                None,
                "Project coordinates are required for local Maven resolution.",
            ),
        );
        return None;
    };
    let mut dependencies = parent
        .as_ref()
        .map(|p| p.dependencies.clone())
        .unwrap_or_default();
    for d in &m.dependencies {
        dependencies.retain(|p| p.group != d.group || p.artifact != d.artifact);
        dependencies.push(d.clone());
    }
    let mut management = parent
        .as_ref()
        .map(|p| p.management.clone())
        .unwrap_or_default();
    for d in &m.management {
        management.retain(|p| p.group != d.group || p.artifact != d.artifact);
        management.push(d.clone());
    }
    let result = EffectiveMaven {
        group,
        artifact,
        version,
        properties: props,
        dependencies,
        management,
    };
    cache.insert(id.into(), result.clone());
    Some(result)
}
fn resolve_maven(
    project: &ServiceProject,
    all: &BTreeMap<String, Maven>,
    effective: &BTreeMap<String, EffectiveMaven>,
    report: &mut WorkspaceReport,
) {
    let Some(raw) = all.get(&project.id) else {
        return;
    };
    for d in &raw.conditional {
        unresolved(
            report,
            project,
            &format!("{}:{}", d.group, d.artifact),
            "profile dependencies are conditional and were not activated",
            d.evidence.clone(),
        );
    }
    let Some(m) = effective.get(&project.id) else {
        return;
    };
    for d in &m.management {
        if d.scope.as_deref() == Some("import") {
            unresolved(
                report,
                project,
                &format!("{}:{}", d.group, d.artifact),
                "imported dependency-management BOM is not evaluated",
                d.evidence.clone(),
            );
        }
    }
    for d in &m.dependencies {
        let key = format!("{}:{}", d.group, d.artifact);
        let (Some(g), Some(a)) = (
            interpolate(&d.group, &m.properties),
            interpolate(&d.artifact, &m.properties),
        ) else {
            unresolved(
                report,
                project,
                &key,
                "dependency coordinates contain unresolved properties",
                d.evidence.clone(),
            );
            continue;
        };
        let candidates: Vec<_> = effective
            .iter()
            .filter(|(_, p)| p.group == g && p.artifact == a)
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let managed = m.management.iter().rev().find(|managed| {
            !managed.unsupported
                && interpolate(&managed.group, &m.properties).as_deref() == Some(&g)
                && interpolate(&managed.artifact, &m.properties).as_deref() == Some(&a)
        });
        let version = d
            .version
            .as_ref()
            .or_else(|| managed.and_then(|m| m.version.as_ref()))
            .and_then(|v| interpolate(v, &m.properties));
        let Some(version) = version else {
            unresolved(
                report,
                project,
                &key,
                "local dependency version is missing or unresolved",
                d.evidence.clone(),
            );
            continue;
        };
        let matches: Vec<_> = candidates
            .into_iter()
            .filter(|(_, p)| p.version == version)
            .collect();
        if matches.len() != 1 {
            unresolved(
                report,
                project,
                &key,
                if matches.is_empty() {
                    "declared version does not match an exact local project version"
                } else {
                    "ambiguous local Maven coordinates"
                },
                d.evidence.clone(),
            );
            continue;
        }
        let scope = interpolate(
            d.scope
                .as_deref()
                .or_else(|| managed.and_then(|v| v.scope.as_deref()))
                .unwrap_or("compile"),
            &m.properties,
        )
        .unwrap_or_default();
        let optional = interpolate(
            d.optional
                .as_deref()
                .or_else(|| managed.and_then(|v| v.optional.as_deref()))
                .unwrap_or("false"),
            &m.properties,
        );
        if !matches!(optional.as_deref(), Some("true" | "false")) {
            unresolved(
                report,
                project,
                &key,
                "optional flag is invalid or unresolved",
                d.evidence.clone(),
            );
            continue;
        }
        if d.unsupported || !matches!(scope.as_str(), "compile" | "runtime" | "provided" | "test") {
            unresolved(
                report,
                project,
                &key,
                "classifier, artifact type or scope is not supported for source-project resolution",
                d.evidence.clone(),
            );
            continue;
        }
        dependency(
            report,
            project,
            matches[0].0,
            if optional.as_deref() == Some("true") {
                "optional"
            } else {
                &scope
            },
            "maven_dependency",
            d.evidence.clone(),
        );
    }
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(value))
}
fn npm_membership<'a>(
    project: &ServiceProject,
    all: &'a BTreeMap<String, Value>,
    report: &'a WorkspaceReport,
) -> Option<&'a str> {
    report
        .projects
        .iter()
        .filter(|p| {
            p.ecosystem == "npm"
                && (p.path == "." || project.path.starts_with(&(p.path.clone() + "/")))
        })
        .filter_map(|owner| {
            let workspaces = all.get(&owner.id)?.get("workspaces")?;
            let patterns = workspaces
                .as_array()
                .or_else(|| workspaces.get("packages")?.as_array())?;
            if owner.id == project.id {
                return Some(owner);
            }
            let relative = if owner.path == "." {
                project.path.as_str()
            } else {
                project.path.strip_prefix(&(owner.path.clone() + "/"))?
            };
            let mut included = false;
            for pattern in patterns.iter().filter_map(Value::as_str) {
                let (exclude, pattern) = pattern
                    .strip_prefix('!')
                    .map(|p| (true, p))
                    .unwrap_or((false, pattern));
                if glob_matches(pattern, relative) {
                    included = !exclude;
                }
            }
            included.then_some(owner)
        })
        .max_by_key(|p| p.path.len())
        .map(|p| p.id.as_str())
}
fn resolve_npm(
    root: &Path,
    project: &ServiceProject,
    all: &BTreeMap<String, Value>,
    report: &mut WorkspaceReport,
) {
    let Some(doc) = all.get(&project.id) else {
        return;
    };
    for (section, scope) in [
        ("dependencies", "compile"),
        ("devDependencies", "test"),
        ("optionalDependencies", "optional"),
        ("peerDependencies", "peer"),
    ] {
        let Some(deps) = doc.get(section).and_then(Value::as_object) else {
            continue;
        };
        for (name, value) in deps {
            if section == "dependencies"
                && doc
                    .get("optionalDependencies")
                    .and_then(Value::as_object)
                    .is_some_and(|optional| optional.contains_key(name))
            {
                continue;
            }
            let Some(value) = value.as_str() else {
                continue;
            };
            let ev = evidence(
                project,
                &format!("/{section}/{name}"),
                None,
                "Explicit local npm dependency declaration.",
            );
            if let Some(path) = value
                .strip_prefix("file:")
                .or_else(|| value.strip_prefix("link:"))
            {
                match path_target(root, project, path, Some("package.json"), "npm", report) {
                    Ok(target)
                        if all
                            .get(&target)
                            .and_then(|v| v.get("name"))
                            .and_then(Value::as_str)
                            == Some(name) =>
                    {
                        dependency(report, project, &target, scope, "npm_file", ev)
                    }
                    Ok(_) => unresolved(
                        report,
                        project,
                        name,
                        "local package name differs from the declared dependency identity",
                        ev,
                    ),
                    Err(reason) => unresolved(report, project, name, reason, ev),
                }
                continue;
            }
            let Some(spec) = value.strip_prefix("workspace:") else {
                continue;
            };
            let membership = npm_membership(project, all, report).map(str::to_owned);
            let Some(membership) = membership else {
                unresolved(
                    report,
                    project,
                    name,
                    "source has no supported package.json workspace membership",
                    ev,
                );
                continue;
            };
            let (target_name, version) = if let Some((alias, version)) =
                spec.rsplit_once('@').filter(|(alias, _)| !alias.is_empty())
            {
                (alias, version)
            } else {
                (name.as_str(), spec)
            };
            let candidates: Vec<_> = if spec.starts_with('.') {
                path_target(root, project, spec, Some("package.json"), "npm", report)
                    .ok()
                    .into_iter()
                    .filter(|id| {
                        all.get(id)
                            .and_then(|v| v.get("name"))
                            .and_then(Value::as_str)
                            == Some(name)
                    })
                    .collect()
            } else {
                report
                    .projects
                    .iter()
                    .filter(|p| {
                        p.ecosystem == "npm"
                            && all
                                .get(&p.id)
                                .and_then(|v| v.get("name"))
                                .and_then(Value::as_str)
                                == Some(target_name)
                    })
                    .map(|p| p.id.clone())
                    .collect()
            };
            let candidates: Vec<_> = candidates
                .into_iter()
                .filter(|id| {
                    report
                        .projects
                        .iter()
                        .find(|p| &p.id == id)
                        .and_then(|p| npm_membership(p, all, report))
                        == Some(membership.as_str())
                })
                .collect();
            if candidates.len() != 1 {
                unresolved(
                    report,
                    project,
                    name,
                    "workspace target is missing, outside membership or ambiguous",
                    ev,
                );
                continue;
            }
            let target = &candidates[0];
            let actual = all
                .get(target)
                .and_then(|v| v.get("version"))
                .and_then(Value::as_str);
            if !spec.starts_with('.')
                && !matches!(version, "*" | "^" | "~")
                && actual != Some(version)
            {
                unresolved(
                    report,
                    project,
                    name,
                    "workspace version is mismatched or uses an unsupported range",
                    ev,
                );
                continue;
            }
            dependency(report, project, target, scope, "npm_workspace", ev);
        }
    }
}
fn resolve_dotnet(
    root: &Path,
    project: &ServiceProject,
    source: &str,
    report: &mut WorkspaceReport,
) {
    let doc = match xml(source) {
        Ok(d) => d,
        Err(message) => {
            report
                .diagnostics
                .push(format!("{}: {message}", project.manifest));
            report.status = "partial".into();
            return;
        }
    };
    for node in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "ProjectReference")
    {
        let target = node.attribute("Include").unwrap_or("");
        let ev = evidence(
            project,
            "/Project/ItemGroup/ProjectReference",
            Some(doc.text_pos_at(node.range().start).row),
            "Explicit MSBuild ProjectReference declaration.",
        );
        if node.ancestors().any(|n| {
            n.attribute("Condition").is_some() || matches!(n.tag_name().name(), "Choose" | "Target")
        }) || node
            .descendants()
            .any(|n| n.attribute("Condition").is_some())
            || node.attribute("Update").is_some()
            || node.attribute("Remove").is_some()
        {
            unresolved(
                report,
                project,
                target,
                "conditional or modified MSBuild ProjectReference is not evaluated",
                ev,
            );
            continue;
        }
        let output = text(node, "ReferenceOutputAssembly")
            .or_else(|| {
                node.attribute("ReferenceOutputAssembly")
                    .map(|v| v.trim().to_owned())
            })
            .unwrap_or_else(|| "true".into());
        let scope = if output.eq_ignore_ascii_case("false") {
            "build"
        } else if output.eq_ignore_ascii_case("true") {
            "compile"
        } else {
            unresolved(
                report,
                project,
                target,
                "ReferenceOutputAssembly is dynamic or is not a supported boolean",
                ev,
            );
            continue;
        };
        match path_target(root, project, target, None, "dotnet", report) {
            Ok(target) => dependency(
                report,
                project,
                &target,
                scope,
                "dotnet_project_reference",
                ev,
            ),
            Err(reason) => unresolved(report, project, target, reason, ev),
        }
    }
}
fn cargo_member(
    project: &ServiceProject,
    owner: &ServiceProject,
    all: &BTreeMap<String, toml::Value>,
) -> bool {
    if project.id == owner.id {
        return true;
    }
    let Some(workspace) = all.get(&owner.id).and_then(|v| v.get("workspace")) else {
        return false;
    };
    let relative = if owner.path == "." {
        Some(project.path.as_str())
    } else {
        project.path.strip_prefix(&(owner.path.clone() + "/"))
    };
    let Some(relative) = relative else {
        return false;
    };
    let matches = |key| {
        workspace
            .get(key)
            .and_then(toml::Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .any(|pattern| glob_matches(pattern, relative))
            })
    };
    matches("members") && !matches("exclude")
}
fn resolve_cargo(
    root: &Path,
    project: &ServiceProject,
    all: &BTreeMap<String, toml::Value>,
    report: &mut WorkspaceReport,
) {
    let Some(doc) = all.get(&project.id) else {
        return;
    };
    let workspace = if let Some(path) = doc
        .get("package")
        .and_then(|p| p.get("workspace"))
        .and_then(toml::Value::as_str)
    {
        local_path(&project.path, path).and_then(|path| {
            report
                .projects
                .iter()
                .find(|p| p.ecosystem == "cargo" && p.path == path)
        })
    } else {
        report
            .projects
            .iter()
            .filter(|p| {
                p.ecosystem == "cargo"
                    && (p.path == "."
                        || project.path.starts_with(&(p.path.clone() + "/"))
                        || project.id == p.id)
                    && all.get(&p.id).and_then(|v| v.get("workspace")).is_some()
            })
            .max_by_key(|p| p.path.len())
    }
    .filter(|owner| cargo_member(project, owner, all))
    .cloned();
    let mut sections = Vec::new();
    for (section, scope) in [
        ("dependencies", "compile"),
        ("dev-dependencies", "test"),
        ("build-dependencies", "build"),
    ] {
        if let Some(values) = doc.get(section).and_then(toml::Value::as_table) {
            sections.push((section.to_owned(), scope, false, values));
        }
    }
    if let Some(targets) = doc.get("target").and_then(toml::Value::as_table) {
        for (target, table) in targets {
            for (section, scope) in [
                ("dependencies", "compile"),
                ("dev-dependencies", "test"),
                ("build-dependencies", "build"),
            ] {
                if let Some(values) = table.get(section).and_then(toml::Value::as_table) {
                    sections.push((format!("target.{target}.{section}"), scope, true, values));
                }
            }
        }
    }
    for (section, scope, conditional, values) in sections {
        for (alias, value) in values {
            let Some(decl) = value.as_table() else {
                continue;
            };
            let ev = evidence(
                project,
                &format!("/{section}/{alias}"),
                None,
                "Explicit Cargo local dependency declaration; build-tool feature/version resolution is not executed.",
            );
            let inherited = decl.get("workspace").and_then(toml::Value::as_bool) == Some(true);
            let (base, dependency_value) = if inherited {
                match workspace.as_ref().and_then(|p| {
                    all.get(&p.id)
                        .and_then(|v| v.get("workspace"))
                        .and_then(|v| v.get("dependencies"))
                        .and_then(|v| v.get(alias))
                        .map(|v| (p, v))
                }) {
                    Some(v) => v,
                    None => {
                        unresolved(
                            report,
                            project,
                            alias,
                            "inherited workspace dependency is unavailable",
                            ev,
                        );
                        continue;
                    }
                }
            } else {
                (project, value)
            };
            let Some(path) = dependency_value.get("path").and_then(toml::Value::as_str) else {
                continue;
            };
            if conditional {
                unresolved(
                    report,
                    project,
                    alias,
                    "target-conditioned Cargo path dependency is not activated",
                    ev,
                );
                continue;
            }
            let name = dependency_value
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(alias);
            let scope = if decl.get("optional").and_then(toml::Value::as_bool) == Some(true) {
                "optional"
            } else {
                scope
            };
            match path_target(root, base, path, Some("Cargo.toml"), "cargo", report) {
                Ok(target)
                    if all
                        .get(&target)
                        .and_then(|v| v.get("package"))
                        .and_then(|v| v.get("name"))
                        .and_then(toml::Value::as_str)
                        == Some(name) =>
                {
                    let mut ev = ev;
                    if inherited {
                        ev.description
                            .push_str(&format!(" Inherited path declared in {}.", base.manifest));
                    }
                    dependency(report, project, &target, scope, "cargo_path", ev);
                }
                Ok(_) => unresolved(
                    report,
                    project,
                    alias,
                    "local Cargo package name differs from declared package identity",
                    ev,
                ),
                Err(reason) => unresolved(report, project, alias, reason, ev),
            }
        }
    }
}
