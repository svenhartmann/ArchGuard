//! The Structurizr workspace (ADR 0013): the service topology as
//! containers, and for one selected service the packages of its source
//! directory as components, with the source connections between them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::{
    ArchguardError, Config, Cut, Graph, Node, NodeId, NodeKind, Service, ServiceTopology,
};
use archguard_report::{C4Components, to_structurizr};

/// The service named `name` (its id, or its name if that is unique).
pub fn c4_service<'t>(t: &'t ServiceTopology, name: &str) -> Result<&'t Service, String> {
    if let Some(s) = t.services.iter().find(|s| s.id == name) {
        return Ok(s);
    }
    let named: Vec<&Service> = t.services.iter().filter(|s| s.name == name).collect();
    match named.as_slice() {
        [s] => Ok(s),
        [] if t.services.is_empty() => Err(format!(
            "unknown service '{name}'; no services were found in the deployment configuration"
        )),
        [] => Err(format!(
            "unknown service '{name}'; known: {}",
            ids(t.services.iter())
        )),
        many => Err(format!(
            "service name '{name}' is ambiguous; use one of: {}",
            ids(many.iter().copied())
        )),
    }
}

fn ids<'a>(services: impl Iterator<Item = &'a Service>) -> String {
    services
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The DSL of the project at `root`; `service` adds its components.
/// A service without a known source directory is an error: its
/// components are unknown, not empty.
pub fn structurizr_workspace(
    root: &Path,
    config: &Config,
    graph: &Graph,
    topology: &ServiceTopology,
    service: Option<&Service>,
) -> Result<String, ArchguardError> {
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("system");
    let components = match service {
        Some(s) => Some(components(root, config, graph, topology, s)?),
        None => None,
    };
    Ok(to_structurizr(name, topology, components.as_ref()))
}

/// The source directory of `s`: its project's path, else its build context.
fn source_dir(t: &ServiceTopology, s: &Service) -> Result<String, ArchguardError> {
    s.project
        .as_ref()
        .and_then(|p| t.projects.iter().find(|x| &x.id == p))
        .map(|p| p.path.clone())
        .or_else(|| s.build_context.clone())
        .ok_or_else(|| {
            ArchguardError::Config(format!(
                "service '{}' has no build context or project path; its components are unknown",
                s.id
            ))
        })
}

/// Components are packages: for each file of the service, its module in
/// the module view (`modules.depth`), or its own directory where the view
/// already shows files. Relations sum the file-level source connections.
fn components<'s>(
    root: &Path,
    config: &Config,
    graph: &Graph,
    t: &ServiceTopology,
    s: &'s Service,
) -> Result<C4Components<'s>, ArchguardError> {
    let dir = source_dir(t, s)?;
    let paths = crate::reference_paths::source_reference_paths(root, config)?;
    let cut = Cut::at_depth(graph, config.modules.depth as usize);
    let by_id: BTreeMap<NodeId, &Node> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    let mut package = BTreeMap::new();
    for file in graph.nodes.iter().filter(|n| n.kind == NodeKind::File) {
        let physical = paths.get(&file.name).unwrap_or(&file.name);
        if dir == "." || physical.starts_with(&prefix) {
            package.insert(file.id, package_of(file, &cut, &by_id).name.clone());
        }
    }
    if package.is_empty() {
        return Err(ArchguardError::Config(format!(
            "no analysed source file lies in '{dir}' of service '{}'; its components are unknown",
            s.id
        )));
    }
    let mut relations = BTreeMap::new();
    for e in &graph.edges {
        if let (Some(a), Some(b)) = (package.get(&e.from), package.get(&e.to))
            && a != b
        {
            *relations.entry((a.clone(), b.clone())).or_insert(0) += e.weight();
        }
    }
    Ok(C4Components {
        service: &s.id,
        modules: package
            .into_values()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        relations: relations.into_iter().map(|((a, b), n)| (a, b, n)).collect(),
    })
}

/// The nearest module in the cut above `file`, else its parent module,
/// else the file itself (a file without a package).
fn package_of<'g>(file: &'g Node, cut: &Cut, by_id: &BTreeMap<NodeId, &'g Node>) -> &'g Node {
    let parent = |n: &Node| n.parent.and_then(|p| by_id.get(&p).copied());
    let mut node = Some(file);
    while let Some(n) = node {
        if cut.contains(n.id) && n.kind == NodeKind::Module {
            return n;
        }
        node = parent(n);
    }
    parent(file).unwrap_or(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "compose.yaml",
            "services:\n  orders:\n    build: ./orders\n    environment:\n      BILLING_API: http://billing:8080/api\n  billing:\n    build: ./billing\n",
        );
        write(
            "orders/api/handler.ts",
            "import { total } from '../domain/order';\nexport const h = total;\n",
        );
        write("orders/domain/order.ts", "export const total = 1;\n");
        write("billing/app.ts", "export const b = 2;\n");
        dir
    }

    #[test]
    fn a_service_has_the_packages_of_its_build_context_as_components() {
        let dir = project();
        let root = dir.path();
        let loaded = crate::config::load_config(None, root).unwrap();
        let run =
            crate::effective::analyze_run_quiet(root, &loaded, crate::pipeline::Level::Module)
                .unwrap();
        let t = crate::services::analyze_services(root).unwrap();
        let orders = c4_service(&t, "orders").unwrap();
        let dsl = structurizr_workspace(root, &run.config, &run.graph, &t, Some(orders)).unwrap();
        assert!(dsl.contains("= component \"api\""), "{dsl}");
        assert!(dsl.contains("= component \"domain\""), "{dsl}");
        assert!(!dsl.contains("component \"app"), "{dsl}");
        assert!(dsl.contains("\"1 source reference\" \"import\""), "{dsl}");
        assert!(dsl.contains("\"HTTP, configured\" \"HTTP\""), "{dsl}");
        let billing = c4_service(&t, "compose.yaml::billing").unwrap();
        let mut elsewhere = billing.clone();
        elsewhere.build_context = Some("docs".into());
        let err = structurizr_workspace(root, &run.config, &run.graph, &t, Some(&elsewhere));
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("components are unknown")
        );
        assert!(
            c4_service(&t, "nope")
                .unwrap_err()
                .contains("unknown service")
        );
    }
}
