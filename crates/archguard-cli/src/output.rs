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

/// Print `text` to stdout, guaranteeing a trailing newline. A reader that
/// closed the pipe (`| head`) is not an error: the command keeps its exit
/// code. Any other write failure is an IO error (exit 3).
pub(crate) fn emit(text: &str) {
    let newline = if text.ends_with('\n') { "" } else { "\n" };
    write_stdout(format_args!("{text}{newline}"));
}

/// `println!` through [`emit`]'s error handling: always one newline, like
/// `println!`, but no panic on a closed pipe.
macro_rules! outln {
    ($($arg:tt)*) => {
        $crate::output::write_stdout(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Piped output is data and stays byte for byte; a terminal gets every
/// control character but `\n` and `\t` escaped, so repository names,
/// imports and JSON fields cannot set titles, clear or colour the screen.
pub(crate) fn write_stdout(args: std::fmt::Arguments) {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        write_raw(format_args!("{}", terminal_safe(&args.to_string())));
    } else {
        write_raw(args);
    }
}

/// [`emit`] for text with ArchGuard's own colour codes whose repository
/// parts are already neutralised (the drawn diagram).
pub(crate) fn emit_drawing(text: &str) {
    let newline = if text.ends_with('\n') { "" } else { "\n" };
    write_raw(format_args!("{text}{newline}"));
}

/// `text` with every control character but `\n` and `\t` written as
/// `\u{..}`.
pub(crate) fn terminal_safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

fn write_raw(args: std::fmt::Arguments) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    match out.write_fmt(args).and_then(|()| out.flush()) {
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => std::process::exit(analysis_error(format!("writing stdout: {err}"))),
        Ok(()) => {}
    }
}

pub(crate) fn usage(msg: impl std::fmt::Display) -> i32 {
    eprintln!(
        "archguard: usage error: {}",
        terminal_safe(&msg.to_string())
    );
    EXIT_USAGE
}

pub(crate) fn analysis_error(err: impl std::fmt::Display) -> i32 {
    eprintln!("archguard: {}", terminal_safe(&err.to_string()));
    EXIT_ERROR
}

#[cfg(test)]
mod tests {
    use super::terminal_safe;

    #[test]
    fn control_chars_are_escaped_visibly_but_lines_and_tabs_stay() {
        let text = "a\x1b]0;X\x07\x1b[31m\u{7f}\u{9b}\nb\tc";
        assert_eq!(
            terminal_safe(text),
            "a\\u{1b}]0;X\\u{7}\\u{1b}[31m\\u{7f}\\u{9b}\nb\tc"
        );
        let json = "{\n  \"name\": \"a\\u001b[31m\"\n}\n";
        assert_eq!(
            terminal_safe(json),
            json,
            "serde's JSON stays byte-identical"
        );
    }
}
