//! HTTP GET with a body size limit, as used for release metadata and assets.

use std::time::Duration;

/// Same cap as the packaging scripts' `MAX_FILE`.
pub const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
/// Cap for release metadata and checksum files.
pub const MAX_METADATA: u64 = 1024 * 1024;
/// Longest wait for the connection, the response head or any single read.
/// A slow but moving download never hits it; a silent server does.
pub const STALL: Duration = Duration::from_secs(30);

/// Downloads `url`, failing when the body exceeds `limit` bytes; the error
/// names the URL.
pub fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    get(url, limit).map_err(|e| format!("{url}: {e}"))
}

/// Like [`download`], but keeps the `ureq` error so callers can tell a 404.
pub fn get(url: &str, limit: u64) -> Result<Vec<u8>, ureq::Error> {
    get_with_stall(url, limit, STALL)
}

/// [`get`] with its own stall timeout, so tests need not wait [`STALL`].
pub fn get_with_stall(url: &str, limit: u64, stall: Duration) -> Result<Vec<u8>, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(stall))
        .timeout_recv_response(Some(stall))
        .timeout_per_read(Some(stall))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            concat!("archguard/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()?;
    response.body_mut().with_config().limit(limit).read_to_vec()
}
