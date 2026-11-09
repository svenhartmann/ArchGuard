//! The CLI's self-update: the archive layout and checksum files are the ones
//! `scripts/release/release_artifacts.py` produces. The archive is checked
//! against `SHA256SUMS` and the binary against `release-info.json` before the
//! running executable is replaced.
use std::io::{Cursor, Read};
use std::path::Path;

use serde_json::Value;

use crate::checksum::sha256_hex;
use crate::http::{self, MAX_DOWNLOAD};
use crate::release::{self, Release};

const SUMS: &str = "SHA256SUMS";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub latest: String,
    pub update_available: bool,
    pub updated: bool,
}

/// Looks up the latest release at `api`; unless `check` is set and a newer
/// release than `current` exists, installs its archive for `target` over the
/// running executable.
pub fn update(api: &str, target: &str, current: &str, check: bool) -> Result<Outcome, String> {
    let release = release::latest(api)?;
    let newer = release.is_newer_than(current)?;
    let latest = release.version.clone();
    if check || !newer {
        return Ok(Outcome {
            latest,
            update_available: newer,
            updated: false,
        });
    }
    let binary = verified_binary(&release, target)?;
    replace_self(&binary)?;
    Ok(Outcome {
        latest,
        update_available: true,
        updated: true,
    })
}

/// The target whose archive a build for `target` installs. Releases ship
/// Linux only as static musl binaries, which also run on glibc systems, so a
/// `cargo install` build (`…-linux-gnu`) updates from those.
fn archive_target(target: &str) -> String {
    match target.strip_suffix("-linux-gnu") {
        Some(arch) => format!("{arch}-linux-musl"),
        None => target.to_string(),
    }
}

fn verified_binary(release: &Release, target: &str) -> Result<Vec<u8>, String> {
    let target = archive_target(target);
    let target = target.as_str();
    let windows = target.contains("windows");
    let latest = release.version.as_str();
    let stem = format!("archguard-{latest}-{target}");
    let name = format!("{stem}{}", if windows { ".zip" } else { ".tar.gz" });
    let expected = release.expected_sha256(SUMS, &name)?;
    let archive = http::download(&release.asset_url(&name)?, MAX_DOWNLOAD)?;
    if sha256_hex(&archive) != expected {
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
    if info["target"] != target || info["version"] != latest {
        return Err("release-info.json does not match the requested target and version".into());
    }
    if info["binary_sha256"] != sha256_hex(&binary).as_str() {
        return Err("binary checksum mismatch against release-info.json".into());
    }
    Ok(binary)
}

/// Reads exactly the `wanted` regular files, in order, from memory; nothing
/// from the archive is extracted onto the filesystem.
fn unpack(archive: &[u8], zip: bool, wanted: &[String]) -> Result<Vec<Vec<u8>>, String> {
    let found = if zip {
        unpack_zip(archive, wanted)?
    } else {
        unpack_tar_gz(archive, wanted)?
    };
    found
        .into_iter()
        .zip(wanted)
        .map(|(data, name)| data.ok_or_else(|| format!("release archive lacks {name}")))
        .collect()
}

fn bad(e: &dyn std::fmt::Display) -> String {
    format!("unreadable release archive: {e}")
}

fn unpack_zip(archive: &[u8], wanted: &[String]) -> Result<Vec<Option<Vec<u8>>>, String> {
    let mut bundle = zip::ZipArchive::new(Cursor::new(archive)).map_err(|e| bad(&e))?;
    let mut found = Vec::with_capacity(wanted.len());
    for name in wanted {
        let mut file = bundle.by_name(name).map_err(|e| bad(&e))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).map_err(|e| bad(&e))?;
        found.push(Some(data));
    }
    Ok(found)
}

fn unpack_tar_gz(archive: &[u8], wanted: &[String]) -> Result<Vec<Option<Vec<u8>>>, String> {
    let mut found: Vec<Option<Vec<u8>>> = vec![None; wanted.len()];
    let mut bundle = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in bundle.entries().map_err(|e| bad(&e))? {
        let mut entry = entry.map_err(|e| bad(&e))?;
        let path = entry
            .path()
            .map_err(|e| bad(&e))?
            .to_string_lossy()
            .into_owned();
        let Some(index) = wanted.iter().position(|w| *w == path) else {
            continue;
        };
        if !entry.header().entry_type().is_file() {
            return Err(format!("{path} is not a regular file"));
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| bad(&e))?;
        found[index] = Some(data);
    }
    Ok(found)
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

#[cfg(test)]
mod tests {
    use super::archive_target;

    #[test]
    fn a_glibc_build_updates_from_the_static_musl_archive() {
        assert_eq!(
            archive_target("x86_64-unknown-linux-gnu"),
            "x86_64-unknown-linux-musl"
        );
        assert_eq!(
            archive_target("aarch64-unknown-linux-gnu"),
            "aarch64-unknown-linux-musl"
        );
        assert_eq!(
            archive_target("aarch64-apple-darwin"),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            archive_target("x86_64-pc-windows-msvc"),
            "x86_64-pc-windows-msvc"
        );
    }
}
