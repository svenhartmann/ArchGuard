//! Exit codes and the small helpers every command prints through.

use serde_json::Value;

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
