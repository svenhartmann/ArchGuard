//! `archguard update` against a local stand-in for the GitHub releases API:
//! a copy of the CLI replaces itself only after both checksums verify.
#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

const TARGET: &str = archguard_update::release::TARGET;
const NEW_BINARY: &[u8] = b"#!/bin/sh\necho archguard-from-release\n";

fn hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A release archive laid out like `scripts/release/release_artifacts.py` packs it.
fn archive(version: &str) -> (String, Vec<u8>) {
    let stem = format!("archguard-{version}-{TARGET}");
    let info = format!(
        "{{\"version\": \"{version}\", \"target\": \"{TARGET}\", \"binary_sha256\": \"{}\"}}\n",
        hex(NEW_BINARY)
    );
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (name, data, mode) in [
        ("README.md", b"readme".as_slice(), 0o644),
        ("archguard", NEW_BINARY, 0o755),
        ("release-info.json", info.as_bytes(), 0o644),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{stem}/{name}"), data)
            .unwrap();
    }
    let bytes = builder.into_inner().unwrap().finish().unwrap();
    (format!("{stem}.tar.gz"), bytes)
}

/// Serves `/repos/svenhartmann/ArchGuard/releases/latest` and the assets it lists.
fn serve(tag: &str, corrupt_sums: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (name, bytes) = archive(tag.trim_start_matches('v'));
    let digest = if corrupt_sums {
        "0".repeat(64)
    } else {
        hex(&bytes)
    };
    let sums = format!("{digest}  {name}\n");
    let latest = format!(
        "{{\"tag_name\": \"{tag}\", \"assets\": [\
         {{\"name\": \"{name}\", \"browser_download_url\": \"{base}/download/{name}\"}},\
         {{\"name\": \"SHA256SUMS\", \"browser_download_url\": \"{base}/download/SHA256SUMS\"}}]}}"
    );
    let archive_path = format!("/download/{name}");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let line = String::from_utf8_lossy(&request);
            let path = line.split_whitespace().nth(1).unwrap_or_default();
            let (status, body): (&str, &[u8]) = match path {
                "/repos/svenhartmann/ArchGuard/releases/latest" => ("200 OK", latest.as_bytes()),
                "/download/SHA256SUMS" => ("200 OK", sums.as_bytes()),
                p if p == archive_path => ("200 OK", &bytes),
                _ => ("404 Not Found", b"".as_slice()),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        }
    });
    base
}

/// A private copy of the CLI, so the replacement never touches cargo's build.
fn installed_copy(dir: &Path) -> PathBuf {
    let exe = dir.join("archguard");
    std::fs::copy(assert_cmd::cargo::cargo_bin!("archguard"), &exe).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    exe
}

fn update(exe: &Path, api: &str, args: &[&str]) -> Output {
    Command::new(exe)
        .arg("update")
        .args(args)
        .env("ARCHGUARD_UPDATE_API", api)
        .output()
        .unwrap()
}

#[test]
fn latest_release_equal_to_current_version_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = installed_copy(dir.path());
    let before = std::fs::read(&exe).unwrap();
    let api = serve(&format!("v{}", env!("CARGO_PKG_VERSION")), false);
    let out = update(&exe, &api, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("up to date"));
    assert_eq!(std::fs::read(&exe).unwrap(), before);
}

#[test]
fn check_reports_newer_release_without_replacing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = installed_copy(dir.path());
    let before = std::fs::read(&exe).unwrap();
    let api = serve("v99.0.0", false);
    let out = update(&exe, &api, &["--check", "--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["latest"], "99.0.0");
    assert_eq!(value["update_available"], true);
    assert_eq!(value["updated"], false);
    assert_eq!(std::fs::read(&exe).unwrap(), before);
}

#[test]
fn newer_release_replaces_the_running_binary_after_checksums_verify() {
    let dir = tempfile::tempdir().unwrap();
    let exe = installed_copy(dir.path());
    let api = serve("v99.0.0", false);
    let out = update(&exe, &api, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("99.0.0"));
    let run = Command::new(&exe).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "archguard-from-release",
        "the replaced file must be the released, executable binary"
    );
}

#[test]
fn checksum_mismatch_keeps_the_installed_binary() {
    let dir = tempfile::tempdir().unwrap();
    let exe = installed_copy(dir.path());
    let before = std::fs::read(&exe).unwrap();
    let api = serve("v99.0.0", true);
    let out = update(&exe, &api, &[]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("checksum"));
    assert_eq!(std::fs::read(&exe).unwrap(), before);
}
