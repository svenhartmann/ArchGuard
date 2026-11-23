use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use archguard_engine::{
    LoadedConfig,
    core::{Graph, MappedArchitectureComponent, TargetArchitectureReport},
};

pub(crate) fn patterns(
    cli: &Cli,
    root: &std::path::Path,
    loaded: &LoadedConfig,
    graph: &Graph,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("patterns supports text and json");
    }
    match archguard_engine::architecture_patterns(root, &loaded.config, graph) {
        Ok(report) => {
            if !cli.quiet {
                if cli.format == Format::Json {
                    emit(&serde_json::to_string_pretty(&report).unwrap());
                } else {
                    outln!("Architecture pattern candidates · {}", report.definition);
                    for p in report.assessments {
                        outln!(
                            "\n{} · {} · {} · basis: {}",
                            p.pattern,
                            p.scope,
                            p.status,
                            p.basis
                        );
                        outln!(
                            "  Evidence: {} · counter-evidence: {}",
                            p.evidence_total,
                            p.counter_evidence_total
                        );
                        for item in p.evidence.iter().chain(&p.counter_evidence) {
                            outln!(
                                "  {}:{} {}",
                                item.file.as_deref().unwrap_or("graph"),
                                item.line.unwrap_or(0),
                                item.message
                            );
                        }
                        for missing in p.missing_evidence {
                            outln!("  Missing: {missing}");
                        }
                    }
                    for note in report.limitations.into_iter().chain(report.diagnostics) {
                        outln!("\n{note}");
                    }
                }
            }
            EXIT_OK
        }
        Err(e) => analysis_error(e),
    }
}

pub(crate) fn run(cli: &Cli, loaded: &LoadedConfig, graph: &Graph) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("architecture supports text and json");
    }
    let report = match archguard_engine::analyze_target_architecture(graph, &loaded.config) {
        Ok(report) => report,
        Err(error) => return analysis_error(error),
    };
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        emit(&report_json(report));
    } else if let Some(report) = report {
        outln!(
            "Target architecture: {} · {} coverage",
            report.name,
            report.status
        );
        outln!(
            "Mapped sources: {}/{} · ambiguous: {} · unmapped: {}",
            report.mapped_nodes,
            report.source_nodes,
            report.ambiguous_nodes,
            report.unmapped_nodes
        );
        outln!(
            "Forbidden component directions: {} · unclassified source dependencies: {}",
            report.violating_dependencies,
            report.unclassified_edges
        );
        outln!(
            "Private source accesses: {} · Coupling limit violations: {}",
            report.violating_interfaces,
            report.coupling_violations
        );
        report.components.iter().for_each(print_component);
        for dependency in report.dependencies {
            outln!(
                "{} → {}: {} · {} source dependencies · {}{} import references",
                dependency.from,
                dependency.to,
                if dependency.allowed {
                    "allowed"
                } else {
                    "forbidden"
                },
                dependency.source_edges,
                if dependency.count_exact { "" } else { "≥" },
                dependency.import_references
            );
            for access in dependency.private_accesses {
                outln!("  Private access: {} → {}", access.from, access.to);
            }
        }
        outln!(
            "Coverage is classification of the configured source scope, not a conformance verdict. Use check for the gate and --format json for mappings and evidence."
        );
    } else {
        outln!("No target architecture configured. Declare architecture.model in archguard.toml.");
    }
    EXIT_OK
}

fn report_json(report: Option<TargetArchitectureReport>) -> String {
    let value = report.map(|r| serde_json::to_value(r).unwrap()).unwrap_or_else(|| {
        serde_json::json!({
            "schema_version": 1,
            "status": "not_configured",
            "message": "Declare architecture.model in archguard.toml to evaluate logical boundaries."
        })
    });
    serde_json::to_string_pretty(&value).unwrap()
}

fn print_component(component: &MappedArchitectureComponent) {
    if let Some(public) = &component.public {
        outln!(
            "{} public interface: {}",
            component.name,
            if public.is_empty() {
                "closed".into()
            } else {
                public.join(", ")
            }
        );
    }
    for (direction, count, limit) in [
        (
            "incoming",
            component.incoming_components.len(),
            component.max_incoming_components,
        ),
        (
            "outgoing",
            component.outgoing_components.len(),
            component.max_outgoing_components,
        ),
    ] {
        if let Some(limit) = limit {
            outln!(
                "{} {direction} components: {count} · limit: {limit}",
                component.name
            );
        }
    }
}
