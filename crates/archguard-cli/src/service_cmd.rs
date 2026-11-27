//! Project and service inventory remains separate from source-import analysis.

use std::collections::BTreeMap;
use std::path::Path;

use archguard_engine::core::{
    ArchguardError, ServiceDatastores, ServiceDependencyKind, ServiceTopology, SharedDatastoreScope,
};

use crate::args::{Cli, Command, Format};
use crate::output::{EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};

/// One path or none: the classic analysis; several paths or `--set`: the merged project set.
fn load(cli: &Cli, root: &Path) -> Result<ServiceTopology, ArchguardError> {
    match &cli.command {
        Command::Services {
            set: Some(file), ..
        } => archguard_engine::read_project_set(file)
            .and_then(|set| archguard_engine::analyze_service_set(&set.projects)),
        Command::Services { paths, .. } if paths.len() > 1 => {
            archguard_engine::analyze_service_set(paths)
        }
        _ => archguard_engine::analyze_services(root),
    }
}

pub(crate) fn run(cli: &Cli, root: &Path, fail_on_found: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("services supports --format text|json");
    }
    let topology = match load(cli, root) {
        Ok(topology) => topology,
        Err(error) => return analysis_error(error),
    };
    let http_cycles = topology
        .cycles
        .iter()
        .filter(|cycle| cycle.kind == ServiceDependencyKind::ConfiguredHttp)
        .count();
    if !cli.quiet {
        if cli.format == Format::Json {
            emit(&serde_json::to_string_pretty(&topology).expect("service topology serializes"));
        } else {
            print_text(&topology, http_cycles);
        }
    }
    if fail_on_found && http_cycles > 0 {
        EXIT_FOUND
    } else {
        EXIT_OK
    }
}

fn print_text(topology: &ServiceTopology, http_cycles: usize) {
    outln!(
        "Services: {} · Compose files: {}",
        topology.services.len(),
        topology.compose_files.len()
    );
    outln!(
        "Monorepo: {} — {}",
        topology.monorepo,
        topology.monorepo_reason
    );
    print_projects(topology);
    outln!("Configured HTTP cycle groups: {http_cycles}");
    outln!(
        "Startup cycle groups: {}",
        topology
            .cycles
            .iter()
            .filter(|cycle| cycle.kind == ServiceDependencyKind::Startup)
            .count()
    );
    outln!(
        "HTTP edges are configured endpoints, not observed runtime calls. Startup edges are depends_on declarations."
    );
    outln!(
        "Supported static input coverage complete: {}",
        topology.complete
    );
    for service in &topology.services {
        outln!(
            "service {}{}",
            service.id,
            service
                .build_context
                .as_ref()
                .map(|context| format!(" (local context: {context})"))
                .unwrap_or_default()
        );
    }
    for edge in &topology.dependencies {
        let kind = kind_label(edge.kind);
        outln!("{kind}: {} -> {}", edge.from, edge.to);
        for evidence in &edge.evidence {
            outln!(
                "  {}{} {}: {}",
                evidence.file,
                evidence
                    .line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default(),
                evidence.pointer,
                evidence.description
            );
        }
    }
    for cycle in &topology.cycles {
        // These are SCC members, not a fabricated ordered cycle path.
        let kind = kind_label(cycle.kind);
        outln!("{kind} cyclic group: {}", cycle.nodes.join(", "));
    }
    print_datastores_and_search(topology);
    for diagnostic in &topology.diagnostics {
        outln!(
            "coverage: {}{}: {}",
            diagnostic.file,
            diagnostic
                .service
                .as_ref()
                .map(|service| format!(" [{service}]"))
                .unwrap_or_default(),
            diagnostic.message
        );
    }
}

fn kind_label(kind: ServiceDependencyKind) -> &'static str {
    match kind {
        ServiceDependencyKind::ConfiguredHttp => "configured HTTP",
        ServiceDependencyKind::Startup => "startup",
        ServiceDependencyKind::HttpCall => "HTTP client",
        ServiceDependencyKind::GrpcCall => "gRPC client",
        ServiceDependencyKind::Event => "event flow (inferred)",
        ServiceDependencyKind::ConfiguredReference => "configured reference",
    }
}

/// Datastores, then Elasticsearch lines; informational, never a gate.
fn print_datastores_and_search(topology: &archguard_engine::core::ServiceTopology) {
    print_datastores(&topology.datastores);
    for line in elasticsearch_lines(&topology.elasticsearch) {
        outln!("{line}");
    }
}

/// Datastore usages and shared datastores; informational, never a gate.
fn print_datastores(datastores: &ServiceDatastores) {
    if datastores.nodes.is_empty() {
        return;
    }
    outln!(
        "Datastores: {} · usages: {}",
        datastores.nodes.len(),
        datastores.usages.len()
    );
    for node in &datastores.nodes {
        outln!(
            "datastore [{}] {}",
            node.engine.as_deref().unwrap_or("unknown"),
            node.id
        );
    }
    for usage in &datastores.usages {
        outln!(
            "datastore usage: {} -> {}{}",
            usage.service,
            usage.datastore,
            usage
                .database
                .as_ref()
                .map(|name| format!(" (database {name})"))
                .unwrap_or_default()
        );
        for evidence in &usage.evidence {
            outln!(
                "  {} {}: {}",
                evidence.file,
                evidence.pointer,
                evidence.description
            );
        }
    }
    let mut tables: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for table in &datastores.tables {
        tables.entry(&table.service).or_default().push(&table.name);
    }
    for (service, names) in tables {
        outln!("tables of {service}: {}", names.join(", "));
    }
    for shared in &datastores.shared {
        match shared.scope {
            SharedDatastoreScope::Database => outln!(
                "shared database{}: {} database {}: {}",
                if shared.disjoint_tables {
                    " (disjoint tables)"
                } else {
                    ""
                },
                shared.datastore,
                shared.database.as_deref().unwrap_or("?"),
                shared.services.join(", ")
            ),
            SharedDatastoreScope::Table => outln!(
                "shared table: {} database {}, tables {}: {}",
                shared.datastore,
                shared.database.as_deref().unwrap_or("?"),
                shared.tables.join(", "),
                shared.services.join(", ")
            ),
            SharedDatastoreScope::Server => outln!(
                "shared server only (database names differ or are unknown): {}: {}",
                shared.datastore,
                shared.services.join(", ")
            ),
        }
    }
    outln!("{}", datastores.note);
}

fn print_projects(topology: &archguard_engine::core::ServiceTopology) {
    outln!("Projects: {}", topology.projects.len());
    for project in &topology.projects {
        outln!(
            "project [{}] {} · {} · {}{}",
            project.kind,
            project.name,
            project.path,
            project.manifest,
            project
                .runtime
                .as_ref()
                .map(|runtime| format!(" · {runtime}"))
                .unwrap_or_default()
        );
    }
}

/// Elasticsearch artifacts, index usages and findings; nothing for an empty model.
fn elasticsearch_lines(model: &archguard_engine::core::ElasticsearchModel) -> Vec<String> {
    let header =
        (!model.projects.is_empty()).then(|| format!("project set: {}", model.projects.join(", ")));
    let artifacts = model.artifacts.iter().map(|a| {
        let origin = if a.origins.is_empty() {
            String::new()
        } else {
            format!(" [from {}]", a.origins.join(", "))
        };
        let flavor = if a.flavor.is_empty() {
            String::new()
        } else {
            format!(" ({})", a.flavor)
        };
        format!(
            "elasticsearch {}: {} ({}){flavor}{origin}",
            a.kind,
            a.name.as_deref().unwrap_or("-"),
            a.address
        )
    });
    let usages = model
        .usages
        .iter()
        .map(|u| format!("index use of {}: {} ({})", u.service, u.index, u.access));
    let findings = model.findings.iter().map(|f| {
        format!(
            "elasticsearch finding {}: {} [{}]",
            f.kind,
            f.subject,
            f.services.join(", ")
        )
    });
    header
        .into_iter()
        .chain(artifacts)
        .chain(usages)
        .chain(findings)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{
        ElasticsearchModel, SearchArtifact, SearchFinding, SearchIndexUsage,
    };

    #[test]
    fn elasticsearch_lines_cover_artifacts_usages_findings() {
        assert!(elasticsearch_lines(&ElasticsearchModel::default()).is_empty());
        let model = ElasticsearchModel {
            artifacts: vec![SearchArtifact {
                flavor: String::new(),
                kind: "index_template".into(),
                name: Some("t1".into()),
                address: "es_index_template.t1".into(),
                scope: "infra".into(),
                patterns: vec![],
                references: vec![],
                evidence: vec![],
                origins: vec![],
                detail: None,
            }],
            usages: vec![SearchIndexUsage {
                service: "api".into(),
                index: "logs-a".into(),
                access: "write".into(),
                evidence: vec![],
            }],
            findings: vec![SearchFinding {
                kind: "undeclared_index".into(),
                subject: "logs-a".into(),
                services: vec!["api".into(), "web".into()],
                evidence: vec![],
            }],
            projects: vec![],
        };
        assert_eq!(
            elasticsearch_lines(&model),
            [
                "elasticsearch index_template: t1 (es_index_template.t1)",
                "index use of api: logs-a (write)",
                "elasticsearch finding undeclared_index: logs-a [api, web]",
            ]
        );
    }
}
