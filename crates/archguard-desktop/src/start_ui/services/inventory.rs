//! The islands of the services World: the port of
//! `repositorySelectableProjects`, `repositoryWorldInventory`,
//! `repositoryDeploymentScope(s)` and `repositoryWorldProjects` in
//! `viewer_repository.js`. An island is a manifest project, one deployment
//! of a project with several, or a service without local sources.

use std::collections::HashSet;

use archguard_engine::core::{Service, ServiceProject, ServiceTopology};

#[derive(Debug, Clone, PartialEq)]
pub struct Island {
    /// The project id, or the service id of a deployment or external service.
    pub id: String,
    /// The web's island key: the project path or `@service/<service id>`.
    pub path: String,
    pub name: String,
    /// The deployment's service name (`displayName`).
    pub display: Option<String>,
    /// `application`, `library`, `tooling`, … (`unknown` for external ones).
    pub kind: String,
    /// The local source project's path; `None` without local sources.
    pub source: Option<String>,
    /// The runtime, else the ecosystem (`npm`, `maven`, …).
    pub tech: Option<String>,
    pub external: bool,
}

impl Island {
    /// The pin label: the service name, else the last path segment.
    pub fn label(&self) -> &str {
        if let Some(d) = &self.display {
            return d;
        }
        if self.external || self.path == "." {
            return &self.name;
        }
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The pin's line after the kind: technology and source path, without
    /// the "." of a project at the repository root.
    pub fn detail(&self) -> String {
        let path = self.source.as_deref().filter(|p| *p != ".");
        let parts: Vec<&str> = [self.tech.as_deref(), path].into_iter().flatten().collect();
        parts.join(" · ")
    }

    /// The name in connection lists (`displayName || name`).
    pub fn title(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.name)
    }
}

/// `repositorySelectableProjects`: no workspaces, relative and unique paths.
fn selectable(t: &ServiceTopology) -> Vec<&ServiceProject> {
    let mut seen = HashSet::new();
    t.projects
        .iter()
        .filter(|p| {
            let path = p.path.as_str();
            let bad = path.is_empty()
                || path.starts_with('/')
                || path.chars().any(|c| c == '\\' || c < ' ')
                || path.split('/').any(|part| part == ".." || part.is_empty());
            p.kind != "workspace" && !bad && seen.insert(path)
        })
        .collect()
}

/// Whether the services World applies: as the web's repository overview,
/// at least two selectable projects, or a monorepo with build units.
pub fn available(t: &ServiceTopology, inventory: &[Island]) -> bool {
    inventory.len() > 1 && (selectable(t).len() > 1 || !crate::documents::build_units(t).is_empty())
}

/// `repositoryWorldInventory`: projects (one island per deployment when a
/// project has several), then services without a local project.
pub fn inventory(t: &ServiceTopology) -> Vec<Island> {
    let projects = selectable(t);
    let mut out = Vec::new();
    for p in &projects {
        let deployments: Vec<&Service> = t
            .services
            .iter()
            .filter(|s| s.project.as_deref() == Some(p.id.as_str()))
            .collect();
        if deployments.len() > 1 {
            out.extend(deployments.iter().map(|s| Island {
                id: s.id.clone(),
                path: service_path(&s.id),
                name: s.name.clone(),
                display: Some(s.name.clone()),
                kind: p.kind.clone(),
                source: Some(p.path.clone()),
                tech: tech(p),
                external: false,
            }));
        } else {
            out.push(Island {
                id: p.id.clone(),
                path: p.path.clone(),
                name: p.name.clone(),
                display: deployments.first().map(|s| s.name.clone()),
                kind: p.kind.clone(),
                source: Some(p.path.clone()),
                tech: tech(p),
                external: false,
            });
        }
    }
    let known: HashSet<&str> = projects.iter().map(|p| p.id.as_str()).collect();
    let external = t.services.iter().filter(|s| {
        let local = s.project.as_deref().is_some_and(|p| known.contains(p));
        !local
            && (!s.compose_file.is_empty()
                || s.id.starts_with("trace:")
                || s.id.starts_with("aspire:"))
    });
    out.extend(external.map(|s| Island {
        id: s.id.clone(),
        path: service_path(&s.id),
        name: s.name.clone(),
        display: None,
        kind: "unknown".into(),
        source: None,
        tech: None,
        external: true,
    }));
    out
}

/// A project's runtime, else its ecosystem.
fn tech(p: &ServiceProject) -> Option<String> {
    [p.runtime.as_deref(), Some(p.ecosystem.as_str())]
        .into_iter()
        .flatten()
        .find(|t| !t.is_empty())
        .map(str::to_string)
}

fn service_path(id: &str) -> String {
    format!("@service/{}", encode_uri_component(id))
}

/// JavaScript's `encodeURIComponent`.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `repositoryDeploymentScope`: a Kubernetes network, the Compose file or
/// the Aspire app host; empty when none.
pub fn deployment_scope(s: &Service) -> String {
    if let Some(n) = s.networks.iter().find(|n| n.starts_with("kubernetes:")) {
        return n.clone();
    }
    if !s.compose_file.is_empty() {
        return s.compose_file.clone();
    }
    if s.id.starts_with("aspire:") {
        return s.id.split("::").next().unwrap_or_default().to_string();
    }
    String::new()
}

/// The deployment variants, sorted.
pub fn deployment_scopes(t: &ServiceTopology) -> Vec<String> {
    let mut out: Vec<String> = t
        .services
        .iter()
        .map(deployment_scope)
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The variant the web starts with when there are several: the root
/// Compose file, else the shallowest, then by name.
pub fn default_deployment(scopes: &[String]) -> Option<String> {
    if scopes.len() < 2 {
        return None;
    }
    const ROOT: [&str; 4] = [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ];
    if let Some(root) = scopes.iter().find(|s| ROOT.contains(&s.as_str())) {
        return Some(root.clone());
    }
    scopes
        .iter()
        .min_by(|a, b| {
            let depth = |s: &str| s.split('/').count();
            depth(a).cmp(&depth(b)).then_with(|| a.cmp(b))
        })
        .cloned()
}

/// `repositoryWorldProjects`: the islands of one deployment variant
/// (`None`: all); unless `all_projects`, only deployed services when there
/// are any.
pub fn shown(
    t: &ServiceTopology,
    inventory: &[Island],
    deployment: Option<&str>,
    all_projects: bool,
) -> Vec<usize> {
    let mut islands: Vec<usize> = (0..inventory.len()).collect();
    if let Some(d) = deployment {
        let scoped = ids(t.services.iter().filter(|s| deployment_scope(s) == d));
        islands.retain(|&i| scoped.contains(inventory[i].id.as_str()));
    }
    let mut deployed = ids(t.services.iter().filter(|s| {
        s.id.starts_with("aspire:") || s.id.starts_with("spring:") || !s.compose_file.is_empty()
    }));
    let any = islands
        .iter()
        .any(|&i| deployed.contains(inventory[i].id.as_str()));
    // Function apps deploy themselves through their build plugin: kept
    // beside a deployment, but they filter nothing on their own.
    deployed.extend(
        t.projects
            .iter()
            .filter(|p| p.runtime.as_deref() == Some("azure-functions"))
            .map(|p| p.id.as_str()),
    );
    if !all_projects && any {
        islands.retain(|&i| inventory[i].external || deployed.contains(inventory[i].id.as_str()));
    }
    islands
}

/// Service ids and their project ids.
fn ids<'a>(services: impl Iterator<Item = &'a Service>) -> HashSet<&'a str> {
    services
        .flat_map(|s| std::iter::once(s.id.as_str()).chain(s.project.as_deref()))
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use archguard_engine::core::{ServiceProject, ServiceTopology};

    pub fn project(path: &str, kind: &str) -> ServiceProject {
        ServiceProject {
            id: format!("project:{path}/pom.xml"),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.into(),
            manifest: format!("{path}/pom.xml"),
            ecosystem: "maven".into(),
            kind: kind.into(),
            runtime: None,
            evidence: Vec::new(),
        }
    }

    pub fn service(id: &str, project: Option<&str>, compose: &str) -> Service {
        Service {
            id: id.into(),
            name: id.rsplit("::").next().unwrap_or(id).to_string(),
            compose_file: compose.into(),
            build_context: None,
            networks: Vec::new(),
            profiles: Vec::new(),
            project: project.map(str::to_string),
        }
    }

    pub fn topology(projects: Vec<ServiceProject>, services: Vec<Service>) -> ServiceTopology {
        ServiceTopology {
            schema_version: 1,
            root: "/repo".into(),
            monorepo: true,
            monorepo_reason: String::new(),
            compose_files: Vec::new(),
            services,
            projects,
            dependencies: Vec::new(),
            cycles: Vec::new(),
            diagnostics: Vec::new(),
            complete: true,
            ..Default::default()
        }
    }

    /// Azure Functions deploy themselves: a local Compose stack beside
    /// them does not hide them (ElasticSearchIK); a plain library does.
    #[test]
    fn function_apps_count_as_deployed() {
        let mut function = project("functions/search", "application");
        function.runtime = Some("azure-functions".into());
        let t = topology(
            vec![function, project("libs/util", "library")],
            vec![service(
                "local/docker-compose.yml::elasticsearch",
                None,
                "local/docker-compose.yml",
            )],
        );
        let inv = inventory(&t);
        let paths: Vec<&str> = shown(&t, &inv, None, false)
            .iter()
            .map(|&i| inv[i].path.as_str())
            .collect();
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths.contains(&"functions/search"), "{paths:?}");
        assert!(!paths.contains(&"libs/util"), "{paths:?}");
        // Without a Compose stack function apps filter nothing on their own.
        let alone = topology(t.projects.clone(), Vec::new());
        assert_eq!(shown(&alone, &inventory(&alone), None, false).len(), 2);
    }

    /// Workspaces and unsafe paths are no islands; a project with two
    /// deployments has one island per deployment; a Compose service
    /// without a project is external.
    #[test]
    fn inventory_follows_the_web() {
        let shop = project("services/shop", "application");
        let t = topology(
            vec![
                shop.clone(),
                project("parent", "workspace"),
                project("../escape", "library"),
                project("shared/core", "library"),
                project("shared/core", "library"),
            ],
            vec![
                service("dev::shop", Some(&shop.id), "dev/compose.yaml"),
                service("prod::shop", Some(&shop.id), "prod/compose.yaml"),
                service("dev::db", None, "dev/compose.yaml"),
                service("orphan", None, ""),
            ],
        );
        let inv = inventory(&t);
        let paths: Vec<&str> = inv.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "@service/dev%3A%3Ashop",
                "@service/prod%3A%3Ashop",
                "shared/core",
                "@service/dev%3A%3Adb"
            ]
        );
        assert_eq!(inv[0].source.as_deref(), Some("services/shop"));
        assert_eq!(
            (inv[0].label(), inv[2].label(), inv[3].label()),
            ("shop", "core", "db")
        );
        assert!(inv[3].external && inv[3].source.is_none());
        let scopes = deployment_scopes(&t);
        assert_eq!(scopes, ["dev/compose.yaml", "prod/compose.yaml"]);
        assert_eq!(
            default_deployment(&scopes).as_deref(),
            Some("dev/compose.yaml")
        );
        // Deployed services only, unless all projects are asked for.
        assert_eq!(shown(&t, &inv, None, false), [0, 1, 3]);
        assert_eq!(shown(&t, &inv, None, true), [0, 1, 2, 3]);
        assert_eq!(shown(&t, &inv, Some("prod/compose.yaml"), false), [1]);
        assert!(available(&t, &inv));
    }

    /// The pin's second line after the kind: the technology and the
    /// source path, but no "." for a project at the repository root.
    #[test]
    fn island_details_name_the_technology_and_the_path() {
        let mut root = project(".", "application");
        root.runtime = Some("node".into());
        let t = topology(vec![root, project("lib/core", "library")], Vec::new());
        let inv = inventory(&t);
        assert_eq!(inv[0].detail(), "node");
        assert_eq!(inv[1].detail(), "maven · lib/core");
        let external = Island {
            tech: None,
            source: None,
            ..inv[0].clone()
        };
        assert_eq!(external.detail(), "");
    }

    #[test]
    fn a_root_compose_file_is_the_default_variant() {
        let scopes = ["compose.yaml".to_string(), "a/compose.yaml".to_string()];
        assert_eq!(default_deployment(&scopes).as_deref(), Some("compose.yaml"));
        assert_eq!(default_deployment(&scopes[..1]), None);
    }
}
