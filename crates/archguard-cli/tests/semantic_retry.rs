//! A failed compiler helper must not be served from the in-process cache: a
//! long-running MCP or `serve` process would otherwise report `unavailable`
//! until a source changes. Each toolchain is replaced by a script that counts
//! its compile attempts and fails.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Ask the same MCP server twice for the call model of `language`; the
/// compile attempts counted after each request.
fn twice(root: &Path, language: &str, env: &[(&str, String)], counter: &Path) -> [usize; 2] {
    let mut command = Command::new(env!("CARGO_BIN_EXE_archguard"));
    command
        .arg("mcp")
        .arg("--root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    let mut read = || {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    send(
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
        "protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"retry","version":"1"}}}),
    );
    read();
    send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let mut counts = [0; 2];
    for (id, count) in counts.iter_mut().enumerate() {
        send(
            json!({"jsonrpc":"2.0","id":id + 1,"method":"tools/call","params":{
            "name":"semantic_model","arguments":{"path":".","kind":"calls","language":language}}}),
        );
        let reply = read();
        assert!(reply.get("result").is_some(), "{reply}");
        *count = attempts(counter);
    }
    drop(send);
    child.kill().unwrap();
    child.wait().unwrap();
    counts
}

fn attempts(counter: &Path) -> usize {
    std::fs::read_to_string(counter).map_or(0, |text| text.lines().count())
}

#[test]
fn failed_go_helper_builds_are_retried() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("go.mod"), "module example.com/app\n\ngo 1.22\n").unwrap();
    std::fs::write(root.join("main.go"), "package main\n\nfunc main() {}\n").unwrap();
    let counter = dir.path().join("attempts");
    let go = dir.path().join("go");
    script(
        &go,
        &format!("echo \"$@\" >> '{}'\nexit 1", counter.display()),
    );
    let [first, second] = twice(
        &root,
        "go",
        &[("ARCHGUARD_GO", go.display().to_string())],
        &counter,
    );
    assert!(first > 0, "the build was attempted");
    assert!(
        second > first,
        "the second request retries the failed build"
    );
}

#[test]
fn failed_csharp_helper_builds_are_retried() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("A.cs"), "class A { void M() {} }\n").unwrap();
    // The smallest SDK layout `sdk()` accepts; compiling through it always fails.
    let home = dir.path().join("dotnet");
    std::fs::create_dir_all(home.join("sdk/9.0.100/Roslyn/bincore")).unwrap();
    for assembly in [
        "csc.dll",
        "Microsoft.CodeAnalysis.dll",
        "Microsoft.CodeAnalysis.CSharp.dll",
    ] {
        std::fs::write(home.join("sdk/9.0.100/Roslyn/bincore").join(assembly), "").unwrap();
    }
    std::fs::create_dir_all(home.join("packs/Microsoft.NETCore.App.Ref/9.0.0/ref/net9.0")).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let counter = dir.path().join("attempts");
    script(
        &bin.join("dotnet"),
        &format!(
            "if [ \"$1\" = --list-sdks ]; then echo '9.0.100 [{}]'; exit 0; fi\necho \"$@\" >> '{}'\nexit 1",
            home.join("sdk").display(),
            counter.display()
        ),
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let [first, second] = twice(&root, "csharp", &[("PATH", path)], &counter);
    assert!(first > 0, "the build was attempted");
    assert!(
        second > first,
        "the second request retries the failed build"
    );
}
