//! HTTP GET with a body size limit, as used for release metadata and assets.

/// Same cap as the packaging scripts' `MAX_FILE`.
pub const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
/// Cap for release metadata and checksum files.
pub const MAX_METADATA: u64 = 1024 * 1024;

/// Downloads `url`, failing when the body exceeds `limit` bytes; the error
/// names the URL.
pub fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    get(url, limit).map_err(|e| format!("{url}: {e}"))
}

/// Like [`download`], but keeps the `ureq` error so callers can tell a 404.
pub fn get(url: &str, limit: u64) -> Result<Vec<u8>, ureq::Error> {
    let mut response = ureq::get(url)
        .header(
            "User-Agent",
            concat!("archguard/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()?;
    response.body_mut().with_config().limit(limit).read_to_vec()
}
