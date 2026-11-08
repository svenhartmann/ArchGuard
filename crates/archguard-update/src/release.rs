//! The latest published release of the repository, read from the GitHub API
//! (`GET /repos/{owner}/{repo}/releases/latest`).

use serde_json::Value;

use crate::checksum;
use crate::http::{self, MAX_METADATA};
use crate::version;

pub const REPOSITORY: &str = "svenhartmann/ArchGuard";
/// The Rust target triple this crate was built for.
pub const TARGET: &str = env!("ARCHGUARD_TARGET");
const DEFAULT_API: &str = "https://api.github.com";

/// `ARCHGUARD_UPDATE_API` if set (mirrors, tests), else `https://api.github.com`.
pub fn api_base() -> String {
    std::env::var("ARCHGUARD_UPDATE_API").unwrap_or_else(|_| DEFAULT_API.to_string())
}

/// One downloadable file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The tag as published, e.g. `v0.3.0`.
    pub tag: String,
    /// The tag without its leading `v`.
    pub version: String,
    /// The release page for people, for a manual download.
    pub page: String,
    pub assets: Vec<Asset>,
}

impl Release {
    /// Whether this release is newer than `current`; fails on a tag that is not `x.y.z`.
    pub fn is_newer_than(&self, current: &str) -> Result<bool, String> {
        let latest = version::parse(&self.version)
            .ok_or_else(|| format!("unsupported release tag {}", self.tag))?;
        let current =
            version::parse(current).ok_or_else(|| format!("unsupported version {current}"))?;
        Ok(latest > current)
    }

    /// The download URL of the asset `name`.
    pub fn asset_url(&self, name: &str) -> Result<String, String> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.url.clone())
            .ok_or_else(|| format!("release {} has no asset {name}", self.tag))
    }

    /// Downloads the sums file `sums_name` and returns the digest it lists for `name`.
    pub fn expected_sha256(&self, sums_name: &str, name: &str) -> Result<String, String> {
        let bytes = http::download(&self.asset_url(sums_name)?, MAX_METADATA)?;
        checksum::expected(&checksum::sums_text(bytes, sums_name)?, sums_name, name)
    }
}

/// Reads the latest published release from the API at `api` (see [`api_base`]).
pub fn latest(api: &str) -> Result<Release, String> {
    let latest_url = format!("{api}/repos/{REPOSITORY}/releases/latest");
    let metadata = http::get(&latest_url, MAX_METADATA).map_err(|e| match e {
        ureq::Error::StatusCode(404) => {
            format!("{REPOSITORY} has no published release yet (drafts are not visible)")
        }
        e => format!("{latest_url}: {e}"),
    })?;
    let release: Value = serde_json::from_slice(&metadata)
        .map_err(|e| format!("unreadable release metadata: {e}"))?;
    parse(&release)
}

fn parse(release: &Value) -> Result<Release, String> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("release metadata has no tag_name")?;
    let assets = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Asset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
            })
        })
        .collect();
    let page = release["html_url"].as_str().map_or_else(
        || format!("https://github.com/{REPOSITORY}/releases/tag/{tag}"),
        str::to_string,
    );
    Ok(Release {
        tag: tag.to_string(),
        version: tag.trim_start_matches('v').to_string(),
        page,
        assets,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn metadata_without_page_links_the_tag_and_skips_incomplete_assets() {
        let release = parse(&json!({
            "tag_name": "v1.2.3",
            "assets": [{"name": "a", "browser_download_url": "http://x/a"}, {"name": "b"}]
        }))
        .unwrap();
        assert_eq!(release.version, "1.2.3");
        assert!(release.page.ends_with("/releases/tag/v1.2.3"));
        assert_eq!(release.asset_url("a").unwrap(), "http://x/a");
        assert_eq!(
            release.asset_url("b").unwrap_err(),
            "release v1.2.3 has no asset b"
        );
        assert!(release.is_newer_than("1.2.2").unwrap());
        assert!(!release.is_newer_than("1.2.3").unwrap());
    }
}
