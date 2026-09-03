use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::upgrade;

/// Download a release asset and return its bytes.
fn download_asset(version: &str, filename: &str) -> Result<Vec<u8>> {
    let url = upgrade::release_asset_url(version, filename);
    let mut buf = Vec::new();
    let mut reader = ureq::get(&url)
        .header("User-Agent", "clockwork-upgrade")
        .call()
        .with_context(|| format!("failed to download {url}"))?
        .into_body()
        .into_reader();
    reader.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Extract the `clockwork` binary from a `.tar.gz` archive.
fn extract_binary(archive_bytes: &[u8]) -> Result<Vec<u8>> {
    let gz = flate2::read::GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.file_name().is_some_and(|n| n == "clockwork") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    bail!("clockwork binary not found in archive")
}

/// Verify SHA-256 checksum of `data` against `sha256.sum` content for the given `filename`.
/// Returns the hex digest on success.
fn verify_checksum(data: &[u8], checksums_txt: &str, filename: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = format!("{:x}", hasher.finalize());

    for line in checksums_txt.lines() {
        // Format: "<hash>  <filename>" or "<hash> <filename>"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == filename {
            if parts[0] == digest {
                return Ok(digest);
            }
            bail!(
                "Checksum mismatch for {filename}.\n  Expected: {}\n  Got:      {digest}",
                parts[0]
            );
        }
    }
    bail!("No checksum entry found for {filename} in sha256.sum")
}

/// Replace the binary at `current_path` atomically.
fn replace_binary(current_path: &Path, new_binary: &[u8]) -> Result<()> {
    let dir = current_path
        .parent()
        .context("cannot determine parent directory of current binary")?;
    let tmp = dir.join(".clockwork-upgrade.tmp");
    fs::write(&tmp, new_binary).context("failed to write temp binary")?;
    set_executable(&tmp)?;
    fs::rename(&tmp, current_path).context("failed to replace binary (atomic rename)")?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o755);
    fs::set_permissions(path, perms).context("failed to set executable permissions")?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn archive_name(target: &str) -> String {
    format!("clockwork-{target}.tar.gz")
}

/// Perform a binary upgrade to the given version. Returns `(install_path, checksum_hex)`.
pub fn execute(version: &str) -> Result<(String, String)> {
    let target = upgrade::detect_target()?;
    let archive_name = archive_name(target);

    let install_path = std::env::current_exe().context("cannot determine current binary path")?;

    // Check write permission
    let parent = install_path
        .parent()
        .context("cannot determine parent directory")?;
    if parent
        .metadata()
        .map(|m| m.permissions().readonly())
        .unwrap_or(true)
    {
        bail!(
            "Cannot write to {}. Try: sudo clockwork upgrade",
            install_path.display()
        );
    }

    println!("Downloading {archive_name}...");
    let archive_bytes = download_asset(version, &archive_name)?;

    println!("Downloading sha256.sum...");
    let checksums_bytes = download_asset(version, "sha256.sum")?;
    let checksums_txt = String::from_utf8(checksums_bytes).context("sha256.sum is not UTF-8")?;

    print!("Verifying checksum... ");
    let digest = verify_checksum(&archive_bytes, &checksums_txt, &archive_name)?;
    println!("verified (SHA-256: {digest})");

    println!("Installing to {}...", install_path.display());
    let binary = extract_binary(&archive_bytes)?;
    replace_binary(&install_path, &binary)?;

    Ok((install_path.display().to_string(), digest))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use flate2::{Compression, write::GzEncoder};

    use super::{archive_name, extract_binary};

    #[test]
    fn names_clockwork_release_archives() {
        assert_eq!(
            archive_name("aarch64-apple-darwin"),
            "clockwork-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn extracts_the_clockwork_binary_from_an_archive() {
        let gzip = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(gzip);
        let body = b"clockwork-binary";
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "clockwork", Cursor::new(body))
            .expect("archive fixture must be valid");
        let gzip = archive.into_inner().expect("archive must finish");
        let bytes = gzip.finish().expect("gzip must finish");

        assert_eq!(
            extract_binary(&bytes).expect("archive must contain clockwork"),
            body
        );
    }
}
