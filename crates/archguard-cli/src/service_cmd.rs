//! Project and service inventory remains separate from source-import analysis.

use std::path::Path;

use archguard_engine::core::{ServiceDatastores, ServiceDependencyKind, SharedDatastoreScope};

use crate::args::{Cli, Format};
use crate::output::{EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};

pub(crate) fn run(cli: &Cli, root: &Path, fail_on_found: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("services supports --format text|json");
    }
    let topology = match archguard_engine::analyze_services(root) {
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
            println!(
                "Services: {} · Compose files: {}",
                topology.services.len(),
                topology.compose_files.len()
            );
            println!(
                "Monorepo: {} — {}",
                topology.monorepo, topology.monorepo_reason
            );
            println!("Projects: {}", topology.projects.len());
            for project in &topology.projects {
                println!(
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
            println!("Configured HTTP cycle groups: {http_cycles}");
            println!(
                "Startup cycle groups: {}",
                topology
                    .cycles
                    .iter()
                    .filter(|cycle| cycle.kind == ServiceDependencyKind::Startup)
                    .count()
            );
            println!(
                "HTTP edges are configured endpoints, not observed runtime calls. Startup edges are depends_on declarations."
            );
            println!(
                "Supported static input coverage complete: {}",
                topology.complete
            );
            for service in &topology.services {
                println!(
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
                println!("{kind}: {} -> {}", edge.from, edge.to);
                for evidence in &edge.evidence {
                    println!(
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
                println!("{kind} cyclic group: {}", cycle.nodes.join(", "));
            }
            print_datastores(&topology.datastores);
            for diagnostic in &topology.diagnostics {
                println!(
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
    }
    if fail_on_found && http_cycles > 0 {
        EXIT_FOUND
    } else {
        EXIT_OK
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

/// Datastore usages and shared datastores; informational, never a gate.
fn print_datastores(datastores: &ServiceDatastores) {
    if datastores.nodes.is_empty() {
        return;
    }
    println!(
        "Datastores: {} · usages: {}",
        datastores.nodes.len(),
        datastores.usages.len()
    );
    for node in &datastores.nodes {
        println!(
            "datastore [{}] {}",
            node.engine.as_deref().unwrap_or("unknown"),
            node.id
        );
    }
    for usage in &datastores.usages {
        println!(
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
            println!(
                "  {} {}: {}",
                evidence.file, evidence.pointer, evidence.description
            );
        }
    }
    for shared in &datastores.shared {
        match shared.scope {
            SharedDatastoreScope::Database => println!(
                "shared database: {} database {}: {}",
                shared.datastore,
                shared.database.as_deref().unwrap_or("?"),
                shared.services.join(", ")
            ),
            SharedDatastoreScope::Server => println!(
                "shared server only (database names differ or are unknown): {}: {}",
                shared.datastore,
                shared.services.join(", ")
            ),
        }
    }
    println!("{}", datastores.note);
}
