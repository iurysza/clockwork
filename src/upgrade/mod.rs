pub mod binary;
pub mod check;

use anyhow::{Result, bail};

/// The current compiled-in version.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const GITHUB_REPO: &str = "iurysza/clockwork";

/// Detect the target triple for the current platform.
pub fn detect_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        (os, arch) => bail!(
            "Clockwork releases support macOS on Apple silicon and Intel Macs; found {os}/{arch}"
        ),
    }
}

/// Fetch the latest release tag from GitHub. Returns the version string without the leading `v`.
pub fn fetch_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let body: serde_json::Value = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "clockwork-upgrade")
        .call()?
        .body_mut()
        .read_json()?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No tag_name in GitHub release response"))?;
    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

/// Compare two semver version strings. Returns true if `latest` is newer than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let parts: Vec<&str> = s.split('.').collect();
        let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(latest) > parse(current)
}

/// Build the download URL for a release asset.
pub fn release_asset_url(version: &str, filename: &str) -> String {
    format!("https://github.com/{GITHUB_REPO}/releases/download/v{version}/{filename}")
}

#[cfg(test)]
mod tests {
    use super::release_asset_url;

    #[test]
    fn uses_the_clockwork_release_url() {
        assert_eq!(
            release_asset_url("0.1.0", "clockwork-aarch64-apple-darwin.tar.gz"),
            "https://github.com/iurysza/clockwork/releases/download/v0.1.0/clockwork-aarch64-apple-darwin.tar.gz"
        );
    }
}
