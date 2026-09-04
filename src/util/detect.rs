use std::process::Command;

use crate::model::config::AgentProfile;

/// A known CLI agent that can be auto-detected and registered.
pub struct KnownAgent {
    pub name: &'static str,
    pub bin: &'static str,
    pub args: &'static [&'static str],
    pub prompt_stdin: bool,
    pub description: &'static str,
}

impl KnownAgent {
    pub fn to_profile(&self) -> AgentProfile {
        AgentProfile {
            bin: binary_on_path(self.bin).unwrap_or_else(|| self.bin.to_string()),
            args: self.args.iter().map(|s| (*s).to_string()).collect(),
            prompt_stdin: self.prompt_stdin,
            cwd: None,
        }
    }
}

/// Known CLI agents for auto-detection. Cursor is excluded (IDE, not a CLI agent).
pub const KNOWN_CLI_AGENTS: &[KnownAgent] = &[
    KnownAgent {
        name: "claude",
        bin: "claude",
        args: &["-p", "--enable-auto-mode"],
        prompt_stdin: false,
        description: "Claude Code (Anthropic)",
    },
    KnownAgent {
        name: "codex",
        bin: "codex",
        args: &["exec", "--full-auto"],
        prompt_stdin: false,
        description: "Codex CLI (OpenAI)",
    },
    KnownAgent {
        name: "gemini",
        bin: "gemini",
        args: &["-p", "--yolo"],
        prompt_stdin: false,
        description: "Gemini CLI (Google)",
    },
    KnownAgent {
        name: "opencode",
        bin: "opencode",
        args: &["run"],
        prompt_stdin: false,
        description: "OpenCode",
    },
    KnownAgent {
        name: "pi",
        bin: "pi",
        args: &["--print", "--mode", "json"],
        prompt_stdin: false,
        description: "Pi Coding Agent",
    },
];

/// Resolve a binary on `PATH`. Profiles keep the absolute path so they also
/// work under launchd's restricted environment.
pub fn binary_on_path(bin: &str) -> Option<String> {
    let output = Command::new("which").arg(bin).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Check if a binary is available on `PATH`.
pub fn is_binary_on_path(bin: &str) -> bool {
    binary_on_path(bin).is_some()
}
