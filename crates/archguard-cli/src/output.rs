//! Exit codes and the small helpers every command prints through.

use serde_json::Value;

use crate::args::Format;

/// Ok (Kap. 9).
pub(crate) const EXIT_OK: i32 = 0;

/// Violations found (`check`, `cycles --fail-on-found`, Kap. 9).
pub(crate) const EXIT_FOUND: i32 = 1;

/// Usage error — clap exits with this code itself on parse failures.
pub(crate) const EXIT_USAGE: i32 = 2;

/// Analysis, IO or config error (Kap. 9).
pub(crate) const EXIT_ERROR: i32 = 3;

/// Pretty-print a [`Value`] via serde_json, with trailing newline.
pub(crate) fn to_pretty_json(value: &Value) -> String {
    let json = serde_json::to_string_pretty(value)
        .expect("SARIF/table documents are plain data and always serializable");
    format!("{json}\n")
}

/// Stable lowercase label for any [`Format`] value — the string the user
/// typed. Used for usage messages of commands with their own subcommand
/// `--format` (`graph`, `dsm`, `simulate`), which must quote the value
/// that was actually rejected, not the global flag's.
pub(crate) fn format_label(format: Format) -> &'static str {
    match format {
        Format::Text => "text",
        Format::Json => "json",
        Format::Mermaid => "mermaid",
        Format::Sarif => "sarif",
        Format::Dot => "dot",
        Format::Html => "html",
        Format::Graphml => "graphml",
        Format::Gexf => "gexf",
        Format::Table => "table",
        Format::PrComment => "pr-comment",
    }
}

/// Print `text` to stdout, guaranteeing a trailing newline.
pub(crate) fn emit(text: &str) {
    if text.ends_with('\n') {
        print!("{text}");
    } else {
        println!("{text}");
    }
}

pub(crate) fn usage(msg: impl std::fmt::Display) -> i32 {
    eprintln!("archguard: usage error: {msg}");
    EXIT_USAGE
}

pub(crate) fn analysis_error(err: impl std::fmt::Display) -> i32 {
    eprintln!("archguard: {err}");
    EXIT_ERROR
}
