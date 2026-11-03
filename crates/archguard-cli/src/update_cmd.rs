//! Self-update from the latest published GitHub release. The archive layout
//! and checksum files are the ones `scripts/release/release_artifacts.py`
//! produces; drafts and pre-releases are invisible to `releases/latest`.
use std::io::{Cursor, Read};
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

const REPOSITORY: &str = "svenhartmann/ArchGuard";
const TARGET: &str = env!("ARCHGUARD_TARGET");
const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Same cap as the packaging script's `MAX_FILE`.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

pub(crate) fn run(cli: &Cli, check: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("update supports text and json");
    }
    match update(check) {
        Ok(outcome) => {
            report(cli.format, &outcome);
            EXIT_OK
        }
        Err(error) => analysis_error(format!("update failed: {error}")),
    }
}

struct Outcome {
    latest: String,
    update_available: bool,
    updated: bool,
}

fn update(check: bool) -> Result<Outcome, String> {
    // Overridable for mirrors and the integration tests.
    let api = std::env::var("ARCHGUARD_UPDATE_API")
        .unwrap_or_else(|_| "https://api.github.com".to_string());
    let latest_url = format!("{api}/repos/{REPOSITORY}/releases/latest");
    let metadata = get(&latest_url, 1024 * 1024).map_err(|e| match e {
        ureq::Error::StatusCode(404) => {
            format!("{REPOSITORY} has no published release yet (drafts are not visible)")
        }
        e => format!("{latest_url}: {e}"),
    })?;
    let release: Value = serde_json::from_slice(&metadata)
        .map_err(|e| format!("unreadable release metadata: {e}"))?;
    let tag = release["tag_name"]
        .as_str()
        .ok_or("release metadata has no tag_name")?;
    let latest = tag.trim_start_matches('v').to_string();
    let newer = version(&latest).ok_or_else(|| format!("unsupported release tag {tag}"))?
        > version(CURRENT).expect("the crate version is x.y.z");
    if check || !newer {
        return Ok(Outcome {
            latest,
            update_available: newer,
            updated: false,
        });
    }

    let windows = TARGET.contains("windows");
    let stem = format!("archguard-{latest}-{TARGET}");
    let name = format!("{stem}{}", if windows { ".zip" } else { ".tar.gz" });
    let asset = |wanted: &str| {
        release["assets"]
            .as_array()
            .and_then(|assets| assets.iter().find(|a| a["name"] == wanted))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("release {tag} has no asset {wanted}"))
    };
    let sums = String::from_utf8(download(&asset("SHA256SUMS")?, 1024 * 1024)?)
        .map_err(|_| "SHA256SUMS is not text")?;
    let expected = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, file)| *file == name)
        .map(|(digest, _)| digest.to_string())
        .ok_or_else(|| format!("SHA256SUMS lists no {name}"))?;
    let archive = download(&asset(&name)?, MAX_DOWNLOAD)?;
    if hex(&archive) != expected {
        return Err(format!("archive checksum mismatch for {name}"));
    }

    let binary_name = if windows {
        "archguard.exe"
    } else {
        "archguard"
    };
    let mut files = unpack(
        &archive,
        windows,
        &[
            format!("{stem}/{binary_name}"),
            format!("{stem}/release-info.json"),
        ],
    )?;
    let info: Value = serde_json::from_slice(&files.pop().expect("two files"))
        .map_err(|e| format!("unreadable release-info.json: {e}"))?;
    let binary = files.pop().expect("two files");
    if info["target"] != TARGET || info["version"] != latest.as_str() {
        return Err("release-info.json does not match the requested target and version".into());
    }
    if info["binary_sha256"] != hex(&binary).as_str() {
        return Err("binary checksum mismatch against release-info.json".into());
    }
    replace_self(&binary)?;
    Ok(Outcome {
        latest,
        update_available: true,
        updated: true,
    })
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    get(url, limit).map_err(|e| format!("{url}: {e}"))
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, ureq::Error> {
    let mut response = ureq::get(url)
        .header(
            "User-Agent",
            concat!("archguard/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()?;
    response.body_mut().with_config().limit(limit).read_to_vec()
}

/// `x.y.z` with an optional `-pre` suffix ignored; `latest` never names a pre-release.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('-').next()?.split('.').map(|p| p.parse().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

fn hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads exactly the `wanted` regular files, in order, from memory; nothing
/// from the archive is extracted onto the filesystem.
fn unpack(archive: &[u8], zip: bool, wanted: &[String]) -> Result<Vec<Vec<u8>>, String> {
    let mut found: Vec<Option<Vec<u8>>> = vec![None; wanted.len()];
    let bad = |e: &dyn std::fmt::Display| format!("unreadable release archive: {e}");
    if zip {
        let mut bundle = zip::ZipArchive::new(Cursor::new(archive)).map_err(|e| bad(&e))?;
        for (slot, name) in found.iter_mut().zip(wanted) {
            let mut file = bundle.by_name(name).map_err(|e| bad(&e))?;
            let mut data = Vec::new();
            file.read_to_end(&mut data).map_err(|e| bad(&e))?;
            *slot = Some(data);
        }
    } else {
        let mut bundle = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in bundle.entries().map_err(|e| bad(&e))? {
            let mut entry = entry.map_err(|e| bad(&e))?;
            let path = entry
                .path()
                .map_err(|e| bad(&e))?
                .to_string_lossy()
                .into_owned();
            if let Some(index) = wanted.iter().position(|w| *w == path) {
                if !entry.header().entry_type().is_file() {
                    return Err(format!("{path} is not a regular file"));
                }
                let mut data = Vec::new();
                entry.read_to_end(&mut data).map_err(|e| bad(&e))?;
                found[index] = Some(data);
            }
        }
    }
    found
        .into_iter()
        .zip(wanted)
        .map(|(data, name)| data.ok_or_else(|| format!("release archive lacks {name}")))
        .collect()
}

/// Stages the new binary next to the running one, then swaps it in atomically
/// (`self-replace` also handles the locked running `.exe` on Windows).
fn replace_self(binary: &[u8]) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| format!("cannot locate archguard: {e}"))?;
    let staged = current.with_file_name(format!(
        "{}.new",
        current.file_name().unwrap_or_default().to_string_lossy()
    ));
    let result = stage(&staged, binary).and_then(|()| {
        self_replace::self_replace(&staged)
            .map_err(|e| format!("cannot replace {}: {e}", current.display()))
    });
    let _ = std::fs::remove_file(&staged);
    result
}

fn stage(path: &Path, binary: &[u8]) -> Result<(), String> {
    std::fs::write(path, binary).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("cannot mark {} executable: {e}", path.display()))?;
    }
    Ok(())
}

fn report(format: Format, outcome: &Outcome) {
    if format == Format::Json {
        let value = json!({
            "current": CURRENT,
            "latest": outcome.latest,
            "target": TARGET,
            "update_available": outcome.update_available,
            "updated": outcome.updated,
        });
        emit(&serde_json::to_string_pretty(&value).expect("update report serializes"));
    } else if outcome.updated {
        println!("Updated archguard {CURRENT} → {}.", outcome.latest);
    } else if outcome.update_available {
        println!(
            "archguard {} is available (installed: {CURRENT}). Run `archguard update` to install it.",
            outcome.latest
        );
    } else {
        println!(
            "archguard {CURRENT} is up to date (latest release: {}).",
            outcome.latest
        );
    }
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn versions_compare_numerically_and_reject_malformed_tags() {
        assert!(version("0.10.0") > version("0.9.9"));
        assert_eq!(version("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(version("1.2"), None);
        assert_eq!(version("1.2.3.4"), None);
        assert_eq!(version("latest"), None);
    }
}
