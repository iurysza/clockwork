use std::path::PathBuf;

use tempfile::TempDir;

/// Create a fresh test environment with its own `CLOCKWORK_HOME`.
pub struct TestEnv {
    pub dir: TempDir,
}

impl TestEnv {
    pub fn new() -> Self {
        Self {
            dir: TempDir::new().expect("failed to create temp dir"),
        }
    }

    pub fn home(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// Get a Command preconfigured with `CLOCKWORK_HOME` and `CLOCKWORK_BACKEND=none`.
    #[allow(deprecated)]
    pub fn cmd(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("clockwork").expect("binary not found");
        cmd.env("CLOCKWORK_HOME", self.home());
        cmd.env("CLOCKWORK_BACKEND", "none");
        cmd
    }
}
