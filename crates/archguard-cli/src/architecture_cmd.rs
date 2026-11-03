use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use archguard_engine::{LoadedConfig, core::Graph};

pub(crate) fn patterns(
    cli: &Cli,
    root: &std::path::Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &archguard_engine::core::Report,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json | Format::Html) {
        return usage("patterns supports text, json and html");
    }
    if cli.format == Format::Html {
        return match crate::review_cmd::html(
            graph,
            loaded,
            None,
            loaded.config.modules.depth,
            &Default::default(),
            &report.findings,
            Some("architecture"),
            None,
        ) {
            Ok(html) => {
                if !cli.quiet {
                    emit(&html);
                }
                EXIT_OK
            }
            Err(e) => analysis_error(e),
        };
    }
    match archguard_engine::architecture_patterns(root, &loaded.config, graph) {
        Ok(report) => {
            if !cli.quiet {
                if cli.format == Format::Json {
                    emit(&serde_json::to_string_pretty(&report).unwrap());
                } else {
                    println!("Architecture pattern candidates · {}", report.definition);
                    for p in report.assessments {
                        println!(
                            "\n{} · {} · {} · basis: {}",
                            p.pattern, p.scope, p.status, p.basis
                        );
                        println!(
                            "  Evidence: {} · counter-evidence: {}",
                            p.evidence_total, p.counter_evidence_total
                        );
                        for item in p.evidence.iter().chain(&p.counter_evidence) {
                            println!(
                                "  {}:{} {}",
                                item.file.as_deref().unwrap_or("graph"),
                                item.line.unwrap_or(0),
                                item.message
                            );
                        }
                        for missing in p.missing_evidence {
                            println!("  Missing: {missing}");
                        }
                    }
                    for note in report.limitations.into_iter().chain(report.diagnostics) {
                        println!("\n{note}");
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
        emit(&serde_json::to_string_pretty(&report.map(|r|serde_json::to_value(r).unwrap()).unwrap_or_else(||serde_json::json!({"schema_version":1,"status":"not_configured","message":"Declare architecture.model in archguard.toml to evaluate logical boundaries."}))).unwrap());
    } else if let Some(report) = report {
        println!(
            "Target architecture: {} · {} coverage",
            report.name, report.status
        );
        println!(
            "Mapped sources: {}/{} · ambiguous: {} · unmapped: {}",
            report.mapped_nodes, report.source_nodes, report.ambiguous_nodes, report.unmapped_nodes
        );
        println!(
            "Forbidden component directions: {} · unclassified source dependencies: {}",
            report.violating_dependencies, report.unclassified_edges
        );
        println!(
            "Private source accesses: {} · Coupling limit violations: {}",
            report.violating_interfaces, report.coupling_violations
        );
        for component in &report.components {
            if let Some(public) = &component.public {
                println!(
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
                    println!(
                        "{} {direction} components: {count} · limit: {limit}",
                        component.name
                    );
                }
            }
        }
        for dependency in report.dependencies {
            println!(
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
                println!("  Private access: {} → {}", access.from, access.to);
            }
        }
        println!(
            "Coverage is classification of the configured source scope, not a conformance verdict. Use check for the gate and --format json for mappings and evidence."
        );
    } else {
        println!(
            "No target architecture configured. Declare architecture.model in archguard.toml."
        );
    }
    EXIT_OK
}
