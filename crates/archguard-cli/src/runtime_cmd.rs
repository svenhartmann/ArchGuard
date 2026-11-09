//! Runtime profile from recorded OTLP traces (ADR 0004). Independent of the
//! import analysis: no archguard.toml or source roots are needed.
use std::path::Path;

use archguard_engine::core::{RuntimeProfile, RuntimeStatement};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, emit, usage};

/// Rows printed per section in text mode; JSON always holds everything.
const TOP: usize = 20;

pub(crate) fn run(cli: &Cli, root: &Path) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("runtime supports --format text|json");
    }
    let profile = archguard_engine::runtime_profile(root);
    if cli.quiet {
        return EXIT_OK;
    }
    match (&profile, cli.format) {
        (_, Format::Json) => {
            emit(&serde_json::to_string_pretty(&profile).expect("runtime profile serializes"))
        }
        (None, _) => println!(
            "No runtime input: expected OTLP JSON Lines in .archguard/runtime/**/*.jsonl or .archguard/otel-traces.json (see probes/CONVENTIONS.md)."
        ),
        (Some(p), _) => print_text(p),
    }
    EXIT_OK
}

fn print_text(p: &RuntimeProfile) {
    let sampling = if p.sampling.is_empty() {
        "unknown".to_owned()
    } else {
        p.sampling.join(", ")
    };
    let state = if p.complete { "complete" } else { "incomplete" };
    println!(
        "Runtime profile: {} spans in {} traces · sampling {sampling} · {state}",
        p.spans, p.traces
    );
    print_endpoints(p);
    print_statements(p);
    if !p.diagnostics.is_empty() {
        println!("\nDiagnostics:");
        for d in &p.diagnostics {
            let at = match d.line {
                Some(line) => format!("{} line {line}", d.file),
                None => d.file.clone(),
            };
            println!("  {at}: {}", d.message);
        }
    }
    println!("\nNote: {}", p.note);
}

fn print_endpoints(p: &RuntimeProfile) {
    let mut endpoints: Vec<_> = p.endpoints.iter().collect();
    endpoints.sort_by(|a, b| {
        b.db_time_ms
            .total_cmp(&a.db_time_ms)
            .then(b.requests.cmp(&a.requests))
    });
    println!("\nEndpoints by DB time ({}):", endpoints.len());
    for e in endpoints.iter().take(TOP) {
        println!(
            "  {} · {} · {} requests, {} errors · p50 {:.1} ms, p95 {:.1} ms · DB calls/request mean {:.1}, max {:.0} · DB {:.1} ms",
            e.service,
            e.name,
            e.requests,
            e.errors,
            e.duration_ms.p50,
            e.duration_ms.p95,
            e.db_calls.mean,
            e.db_calls.max,
            e.db_time_ms
        );
    }
    more(endpoints.len());
}

fn total_ms(s: &RuntimeStatement) -> f64 {
    s.exec_ms.mean * s.calls as f64
}

fn print_statements(p: &RuntimeProfile) {
    let mut statements: Vec<_> = p.statements.iter().collect();
    statements.sort_by(|a, b| {
        total_ms(b)
            .total_cmp(&total_ms(a))
            .then(b.calls.cmp(&a.calls))
    });
    println!("\nStatements by DB time ({}):", statements.len());
    for s in statements.iter().take(TOP) {
        let read = s
            .read_ms_total
            .map(|r| format!(", read {r:.1} ms"))
            .unwrap_or_default();
        println!(
            "  {} · {} · {} calls, {} errors, max {} per request · mean {:.1} ms, max {:.1} ms{read}",
            s.service,
            s.db_system.as_deref().unwrap_or("db"),
            s.calls,
            s.errors,
            s.max_calls_per_request,
            s.exec_ms.mean,
            s.exec_ms.max
        );
        println!("    {}", s.text.as_deref().unwrap_or(&s.key));
        for site in &s.call_sites {
            let location = match (&site.file, site.line) {
                (Some(file), Some(line)) => format!(" ({file}:{line})"),
                (Some(file), None) => format!(" ({file})"),
                _ => String::new(),
            };
            let function = site.function.as_deref().unwrap_or("?");
            println!("    at {function}{location} × {}", site.calls);
        }
    }
    more(statements.len());
}

fn more(total: usize) {
    if total > TOP {
        println!("  … {} more (use --format json)", total - TOP);
    }
}
