//! What arc42 chapter 3 (context) reads from the source, for every
//! language (ADR 0006): programs the code starts (`Command::new("git")`,
//! `subprocess.run(["node", …])`, `exec.Command`, `ProcessBuilder`,
//! `Process.Start`, `spawn`, …) and hosts it addresses over HTTP (URLs in
//! files that use an HTTP client; plain documentation links elsewhere do
//! not count). Tests, build output and vendored code are skipped.

use std::collections::BTreeMap;
use std::path::Path;

use archguard_report::Arc42Context;

/// Calls that start a program, with the program as first string argument.
const SPAWNS: [&str; 17] = [
    "Command::new(",
    "exec.Command(",
    "ProcessBuilder(",
    "Process.Start(",
    "subprocess.run(",
    "subprocess.Popen(",
    "subprocess.call(",
    "subprocess.check_call(",
    "subprocess.check_output(",
    "spawn(",
    "spawnSync(",
    "execFile(",
    "execFileSync(",
    "proc_open(",
    "shell_exec(",
    "Runtime.getRuntime().exec(",
    "os.system(",
];

/// Signs that a file talks HTTP itself.
const CLIENTS: [&str; 18] = [
    "reqwest",
    "ureq::",
    "hyper::",
    "requests.",
    "httpx",
    "urllib.request",
    "fetch(",
    "axios",
    "HttpClient",
    "http.Get(",
    "http.NewRequest",
    "okhttp",
    "RestTemplate",
    "WebClient",
    "curl_init",
    "Guzzle",
    "script.src",
    "<script src",
];

const SOURCES: [&str; 13] = [
    "rs", "py", "go", "java", "kt", "cs", "ts", "tsx", "js", "mjs", "cjs", "php", "rb",
];
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 1 << 20;
const MAX_EACH: usize = 10;

/// Skipped: tests and code that is not the project's own.
fn skipped(path: &str) -> bool {
    let p = path.replace('\\', "/");
    [
        "/tests/",
        "/test/",
        "/__tests__/",
        "/node_modules/",
        "/vendor/",
        "/target/",
        "/dist/",
        "/build/",
    ]
    .iter()
    .any(|d| p.contains(d))
        || [
            ".spec.",
            ".test.",
            "_test.go",
            "Test.java",
            "Tests.cs",
            "test_",
        ]
        .iter()
        .any(|t| p.rsplit('/').next().is_some_and(|f| f.contains(t)))
}

#[tracing::instrument(skip_all)]
pub(crate) fn scan(root: &Path) -> Arc42Context {
    let mut programs: BTreeMap<String, usize> = BTreeMap::new();
    let mut hosts: BTreeMap<String, usize> = BTreeMap::new();
    let files = ignore::WalkBuilder::new(root)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| SOURCES.contains(&x))
        })
        .filter(|e| {
            !skipped(&format!(
                "/{}",
                e.path().strip_prefix(root).unwrap_or(e.path()).display()
            ))
        })
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES))
        .take(MAX_FILES);
    for entry in files {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        // Rust keeps unit tests in the file, after `#[cfg(test)]`.
        let tests = ["#[cfg(test)]", "#[cfg(all(test"]
            .iter()
            .filter_map(|marker| text.find(marker))
            .min();
        let text = match tests {
            Some(at) if entry.path().extension().is_some_and(|x| x == "rs") => &text[..at],
            _ => text.as_str(),
        };
        for name in started(text) {
            *programs.entry(name).or_default() += 1;
        }
        if CLIENTS.iter().any(|c| text.contains(c)) {
            for host in addressed(text) {
                *hosts.entry(host).or_default() += 1;
            }
        }
    }
    Arc42Context {
        programs: top(programs),
        hosts: top(hosts),
    }
}

/// Most files first, then by name; at most [`MAX_EACH`].
fn top(counts: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list.truncate(MAX_EACH);
    list
}

/// Program names started in `text`, each once.
fn started(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for marker in SPAWNS {
        for (at, _) in text.match_indices(marker) {
            if let Some(name) = first_literal(&text[at + marker.len()..]).and_then(program)
                && !out.contains(&name)
            {
                out.push(name);
            }
        }
    }
    out
}

/// The string literal opening the argument list (after `[` or `new`).
fn first_literal(rest: &str) -> Option<&str> {
    let rest = rest.trim_start().trim_start_matches('[').trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let body = &rest[1..];
    body.find(quote).map(|end| &body[..end])
}

/// `"/usr/bin/git status"` → `git`; nothing for paths built at runtime.
fn program(literal: &str) -> Option<String> {
    let word = literal.split_whitespace().next()?;
    let name = word.rsplit(['/', '\\']).next()?.trim_end_matches(".exe");
    let plain = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    plain.then(|| name.to_string())
}

/// Hosts of `http://` and `https://` URLs in `text`, each once.
fn addressed(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for scheme in ["http://", "https://"] {
        for (at, _) in text.match_indices(scheme) {
            // A link in Markdown or HTML text points readers, not the program.
            let before = &text[..at];
            if before.ends_with("](") || before.ends_with("href=\"") || before.ends_with("href='") {
                continue;
            }
            let rest = &text[at + scheme.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
                .unwrap_or(rest.len());
            let host = rest[..end].to_ascii_lowercase();
            // Reserved for tests and documentation (RFC 2606, RFC 6761).
            let reserved = [".invalid", ".test", ".example", ".localhost"]
                .iter()
                .any(|t| host.ends_with(t));
            // XML namespaces name a vocabulary, not a server.
            let namespace = host == "www.w3.org" || host.starts_with("schemas.");
            let local = reserved
                || namespace
                || [
                    "localhost",
                    "127.0.0.1",
                    "0.0.0.0",
                    "example.com",
                    "example.org",
                ]
                .contains(&host.as_str());
            if host.contains('.') && !local && !out.contains(&host) {
                out.push(host);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_programs_and_addressed_hosts_across_languages() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "src/a.rs",
            r#"let o = Command::new("git").arg("log"); let n = Command::new(path);"#,
        );
        write(
            "src/b.py",
            "subprocess.run(['node', 'x.js'])\nsubprocess.run([\"git\", \"status\"])",
        );
        write(
            "cmd/c.go",
            r#"exec.Command("/usr/local/bin/rust-analyzer", "scip")"#,
        );
        write(
            "web/d.ts",
            r#"const r = await fetch("https://api.github.com/repos/x"); // https://localhost:3000"#,
        );
        write(
            "docs/e.rs",
            r#"const LINK: &str = "https://arc42.org/overview";"#,
        );
        write("tests/t.rs", r#"Command::new("evil")"#);
        write("src/f.spec.ts", r#"spawn("evil2")"#);
        write(
            "src/g.rs",
            "fn run() { Command::new(\"cargo\"); }\n#[cfg(test)]\nmod tests { fn t() { Command::new(\"evil3\"); } }",
        );
        write(
            "web/h.js",
            r#"fetch("https://cdn.example.net/x"); const a = '[docs](https://docs.rs/x)'; fetch("https://viewer.invalid/y");"#,
        );
        let c = scan(dir.path());
        assert_eq!(
            c.programs,
            [
                ("git".to_string(), 2),
                ("cargo".into(), 1),
                ("node".into(), 1),
                ("rust-analyzer".into(), 1)
            ],
            "Rust unit tests after #[cfg(test)] do not count"
        );
        assert_eq!(
            c.hosts,
            [
                ("api.github.com".to_string(), 1),
                ("cdn.example.net".into(), 1)
            ],
            "documentation links, Markdown links and reserved domains do not count"
        );
    }
}
