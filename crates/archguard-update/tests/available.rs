//! `desktop::available` and the downloads against the local API stub.
mod stub;

use std::collections::HashMap;

use archguard_update::checksum::sha256_hex;
use archguard_update::desktop::{self, How, Installed};
use archguard_update::http;

const TARGET: &str = "x86_64-unknown-linux-gnu";
const PACKAGE: &[u8] = b"new appimage";

/// A release `tag` with the Linux AppImage and `SHA256SUMS-desktop`.
fn release(tag: &str, digest: Option<String>) -> String {
    let version = tag.trim_start_matches('v').to_string();
    let name = format!("ArchGuard-{version}-{TARGET}.AppImage");
    let digest = digest.unwrap_or_else(|| sha256_hex(PACKAGE));
    stub::serve(|base| {
        HashMap::from([
            (
                stub::LATEST.to_string(),
                stub::latest_json(base, tag, &[&name, desktop::SUMS]),
            ),
            (format!("/download/{name}"), PACKAGE.to_vec()),
            (
                "/download/SHA256SUMS-desktop".to_string(),
                format!("0000  other.dmg\n{digest}  {name}\n").into_bytes(),
            ),
        ])
    })
}

#[test]
fn newer_release_names_the_target_package_and_its_listed_digest() {
    let api = release("v0.9.0", None);
    let update = desktop::available(&api, "0.2.1", TARGET).unwrap().unwrap();
    assert_eq!(update.version, "0.9.0");
    assert_eq!(update.package, format!("ArchGuard-0.9.0-{TARGET}.AppImage"));
    assert_eq!(update.url, format!("{api}/download/{}", update.package));
    assert_eq!(update.sha256, sha256_hex(PACKAGE));
    assert_eq!(update.page, "https://example.invalid/v0.9.0");
}

#[test]
fn same_or_older_release_is_no_update() {
    let api = release("v0.2.1", None);
    assert_eq!(desktop::available(&api, "0.2.1", TARGET).unwrap(), None);
    assert_eq!(desktop::available(&api, "0.10.0", TARGET).unwrap(), None);
}

#[test]
fn target_without_desktop_package_is_an_error() {
    let api = release("v0.9.0", None);
    let error = desktop::available(&api, "0.2.1", "aarch64-unknown-linux-musl").unwrap_err();
    assert!(error.contains("no desktop package"), "{error}");
}

#[test]
fn no_published_release_is_reported_as_such() {
    let api = stub::serve(|_| HashMap::new());
    let error = desktop::available(&api, "0.2.1", TARGET).unwrap_err();
    assert!(error.contains("no published release yet"), "{error}");
}

#[test]
fn download_rejects_a_body_over_the_limit() {
    let api = release("v0.9.0", None);
    let url = format!("{api}/download/ArchGuard-0.9.0-{TARGET}.AppImage");
    assert_eq!(http::download(&url, 1024).unwrap(), PACKAGE);
    assert!(http::download(&url, 4).is_err());
}

#[test]
fn available_download_and_install_replace_the_appimage_end_to_end() {
    let api = release("v0.9.0", None);
    let update = desktop::available(&api, "0.2.1", TARGET).unwrap().unwrap();
    let bytes = http::download(&update.url, http::MAX_DOWNLOAD).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("ArchGuard.AppImage");
    std::fs::write(&image, b"old appimage").unwrap();
    let installed = desktop::install(&bytes, &update, &How::AppImage(Some(image.clone())));
    assert_eq!(
        installed.unwrap(),
        Installed::Replaced {
            path: image.clone()
        }
    );
    assert_eq!(std::fs::read(&image).unwrap(), PACKAGE);
}

#[test]
fn a_wrong_listed_digest_fails_the_install_and_keeps_the_appimage() {
    let api = release("v0.9.0", Some("0".repeat(64)));
    let update = desktop::available(&api, "0.2.1", TARGET).unwrap().unwrap();
    let bytes = http::download(&update.url, http::MAX_DOWNLOAD).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("ArchGuard.AppImage");
    std::fs::write(&image, b"old appimage").unwrap();
    let error = desktop::install(&bytes, &update, &How::AppImage(Some(image.clone())));
    assert!(error.unwrap_err().contains("checksum mismatch"));
    assert_eq!(std::fs::read(&image).unwrap(), b"old appimage");
}
