use std::path::PathBuf;

use anyhow::{Result, bail};

/// Expand a leading `~` and require an existing directory.
pub fn resolve_directory(input: &str) -> Result<PathBuf> {
    if input.is_empty() {
        bail!("working directory must not be empty");
    }

    let path = if input == "~" {
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?
    } else if let Some(rest) = input.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?
            .join(rest)
    } else {
        PathBuf::from(input)
    };

    if !path.is_dir() {
        bail!("working directory is not a directory: {}", path.display());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_existing_directories_and_rejects_missing_paths() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert_eq!(
            resolve_directory(dir.path().to_str().unwrap()).unwrap(),
            dir.path()
        );
        assert!(resolve_directory("/definitely/not/a/clockwork-directory").is_err());
    }
}
