//! Environment check: which external packages this machine needs for full features.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, emit, usage};
use std::process::{Command, Stdio};

struct ToolReport {
    name: &'static str,
    feature: &'static str,
    status: &'static str,
    version: Option<String>,
    hint: Option<String>,
}

/// Version probes are read-only executions of `--version`; nothing in doctor
/// analyses a project or runs user code.
fn probe(executable: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // java -version prints to stderr; both streams are probed the same way.
    // The JVM launcher prefixes noise like "Picked up JAVA_TOOL_OPTIONS: …".
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        text.lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("Picked up "))
            .unwrap_or_default()
            .to_string(),
    )
}

fn install_hint(os: &str, tool: &str) -> String {
    let (macos, windows, linux): (&str, &str, &str) = match tool {
        "git" => (
            "brew install git",
            "winget install Git.Git",
            "apt install git (Debian/Ubuntu) or dnf install git (Fedora/RHEL)",
        ),
        "java" => (
            "brew install --cask temurin (or: brew install openjdk)",
            "winget install EclipseAdoptium.Temurin.21.JDK",
            "apt install default-jdk (Debian/Ubuntu) or dnf install java-21-openjdk-devel",
        ),
        "dotnet" => (
            "brew install --cask dotnet-sdk",
            "winget install Microsoft.DotNet.SDK.8",
            "install dotnet-sdk from the official Microsoft package feed",
        ),
        _ => (
            "brew install node",
            "winget install OpenJS.NodeJS.LTS",
            "apt install nodejs npm (Debian/Ubuntu) or dnf install nodejs",
        ),
    };
    match os {
        "macos" => macos.to_string(),
        "windows" => windows.to_string(),
        "linux" => linux.to_string(),
        other => format!("install {tool} with the package manager of {other}"),
    }
}

pub(crate) fn run(cli: &Cli) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("doctor supports text and json");
    }
    let os = std::env::consts::OS;
    let checks: [(&str, &[&str], &str); 4] = [
        (
            "git",
            &["--version"],
            "Git history evidence: review priorities, co-change pairs, quality gates and snapshots",
        ),
        (
            "java",
            &["-version"],
            "Java call analysis via the javac scanner; without it Java imports are still analyzed",
        ),
        (
            "dotnet",
            &["--version"],
            "C# call analysis via the Roslyn scanner; without it namespaces and imports are still analyzed",
        ),
        (
            "node",
            &["--version"],
            "TypeScript/JavaScript call analysis; the analyzed project also needs npm install --save-dev typescript, and running it needs ARCHGUARD_TRUST_PROJECT_COMPILER=1",
        ),
    ];
    let tools: Vec<ToolReport> = checks
        .iter()
        .map(|(name, args, feature)| match probe(name, args) {
            Some(version) => ToolReport {
                name,
                feature,
                status: "available",
                version: Some(version),
                hint: None,
            },
            None => ToolReport {
                name,
                feature,
                status: "missing",
                version: None,
                hint: Some(install_hint(os, name)),
            },
        })
        .collect();
    if cli.format == Format::Json {
        let value = serde_json::json!({
            "os": os,
            "tools": tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "feature": tool.feature,
                "status": tool.status,
                "version": tool.version,
                "hint": tool.hint,
            })).collect::<Vec<_>>(),
        });
        emit(&serde_json::to_string_pretty(&value).expect("doctor report serializes"));
    } else {
        println!("Environment for full archguard features (OS: {os})");
        for tool in &tools {
            match &tool.version {
                Some(version) => {
                    println!("{}: available ({version}) — {}", tool.name, tool.feature)
                }
                None => println!(
                    "{}: MISSING — {}; install: {}",
                    tool.name,
                    tool.feature,
                    tool.hint.as_deref().unwrap_or_default()
                ),
            }
        }
        println!("Missing optional tools only reduce analysis depth; archguard keeps working.");
    }
    EXIT_OK
}
