use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

/// Version embedded in installed skill files for tracking.
const SKILL_VERSION: &str = env!("CARGO_PKG_VERSION");

// Embed the same skill files for every supported installation target.
const SKILL_MD: &str = include_str!("../../skills/clockwork/SKILL.md");
const REFERENCE_MD: &str = include_str!("../../skills/clockwork/reference.md");

/// Known agent targets for skill installation.
const KNOWN_AGENTS: &[&str] = &["claude", "codex", "cursor", "gemini", "opencode", "pi"];

#[allow(clippy::fn_params_excessive_bools)]
pub fn execute(
    agent: Option<&str>,
    all: bool,
    force: bool,
    dry_run: bool,
    list: bool,
    json_output: bool,
) -> Result<()> {
    if list {
        return list_skills(json_output);
    }

    let opts = InstallOpts {
        agent,
        all,
        force,
        dry_run,
    };
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    let results = collect_install(&opts, &home)?;

    if json_output {
        // Keep skills and agent detection in one parseable JSON document.
        let mut doc = serde_json::json!({ "skills": skill_results_json(&results) });
        if !dry_run {
            let agents = super::agent::detect_agents_json(force)?;
            doc["agents"] = agents["agents"].clone();
            doc["default_agent"] = agents["default_agent"].clone();
        }
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        if results.is_empty() {
            println!("No supported agents detected.");
            println!("Install one of: Claude Code, Codex, Cursor, Gemini CLI, OpenCode, or Pi.");
            println!("Or specify an agent directly: clockwork setup --agent claude");
            println!("Or install for all supported agents: clockwork setup --all");
        } else {
            print_results_human(&results, dry_run, &home);
        }
        // Also register detected agent profiles for `--prompt` jobs.
        if !dry_run {
            println!();
            super::agent::detect_agents(force, false)?;
        }
    }

    Ok(())
}

#[allow(clippy::struct_excessive_bools)]
struct InstallOpts<'a> {
    agent: Option<&'a str>,
    all: bool,
    force: bool,
    dry_run: bool,
}

/// Write skill files and return results without printing.
/// The caller chooses human-readable or JSON output.
fn collect_install(opts: &InstallOpts<'_>, home: &Path) -> Result<Vec<SkillResult>> {
    let targets = resolve_targets(opts)?;
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let mut results: Vec<SkillResult> = targets
        .iter()
        .map(|&a| install_for_agent(a, home, opts.force, opts.dry_run))
        .collect();

    // Also report undetected agents when not targeting a specific one.
    if opts.agent.is_none() {
        for &name in KNOWN_AGENTS {
            if !targets.contains(&name) {
                results.push(SkillResult {
                    agent: name.to_string(),
                    path: None,
                    status: "not_detected".to_string(),
                });
            }
        }
    }

    Ok(results)
}

fn resolve_targets<'a>(opts: &InstallOpts<'a>) -> Result<Vec<&'a str>> {
    if let Some(name) = opts.agent {
        if !KNOWN_AGENTS.contains(&name) {
            bail!(
                "Unknown agent '{name}'. Valid agents: {}",
                KNOWN_AGENTS.join(", ")
            );
        }
        return Ok(vec![name]);
    }

    // --all also installs skills for agents that are not installed yet.
    if opts.all {
        return Ok(KNOWN_AGENTS.to_vec());
    }

    Ok(KNOWN_AGENTS
        .iter()
        .filter(|a| is_agent_detected(a))
        .copied()
        .collect())
}

fn skill_results_json(results: &[SkillResult]) -> Vec<serde_json::Value> {
    results
        .iter()
        .map(|r| {
            serde_json::json!({
                "agent": r.agent,
                "path": r.path,
                "status": r.status,
            })
        })
        .collect()
}

fn print_results_json(results: &[SkillResult]) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&skill_results_json(results))?
    );
    Ok(())
}

fn print_results_human(results: &[SkillResult], dry_run: bool, home: &Path) {
    let action = if dry_run {
        "Would install"
    } else {
        "Installing"
    };
    println!("{action} clockwork skills...");

    for r in results {
        let icon = match r.status.as_str() {
            "installed" | "would_install" => "+",
            "already_installed" => "~",
            "not_detected" => "-",
            _ => "!",
        };
        let path_str = r
            .path
            .as_deref()
            .map(|p| format!("  {p}"))
            .unwrap_or_default();
        let status_label = match r.status.as_str() {
            "installed" => "installed",
            "would_install" => "would install",
            "already_installed" => "already installed",
            "not_detected" => "not detected",
            _ => &r.status,
        };
        println!("  {icon} {:<12} [{status_label}]{path_str}", r.agent);
    }

    let installed_count = results
        .iter()
        .filter(|r| r.status == "installed" || r.status == "would_install")
        .count();
    if installed_count > 0 && !dry_run {
        println!();
        println!(
            "{installed_count} skill(s) installed. Your agents now know how to use clockwork."
        );
        println!("Skills are version-matched to clockwork v{SKILL_VERSION}.");
    }

    // Post-install notes for agents that need manual config.
    if !dry_run {
        for r in results {
            if r.status == "installed" || r.status == "already_installed" {
                print_post_install_note(&r.agent, r.path.as_deref(), home);
            }
        }
    }
}

fn install_for_agent(agent: &str, home: &Path, force: bool, dry_run: bool) -> SkillResult {
    let (target_files, target_dir) = skill_target(agent, home);

    let primary = target_dir.join(target_files[0].0);
    if primary.exists() && !force {
        return SkillResult {
            agent: agent.to_string(),
            path: Some(display_path(&primary, home)),
            status: "already_installed".to_string(),
        };
    }

    if dry_run {
        return SkillResult {
            agent: agent.to_string(),
            path: Some(display_path(&primary, home)),
            status: "would_install".to_string(),
        };
    }

    if let Err(e) = fs::create_dir_all(&target_dir) {
        return SkillResult {
            agent: agent.to_string(),
            path: None,
            status: format!("error: {e}"),
        };
    }

    for (filename, content) in &target_files {
        let path = target_dir.join(filename);
        // Append (don't prepend) the version marker: SKILL.md leads with YAML
        // frontmatter that the native skill format requires at the very top, so
        // a comment before it would hide the skill's name/description.
        let versioned = format!("{content}\n<!-- clockwork-skill v{SKILL_VERSION} -->\n");
        if let Err(e) = fs::write(&path, versioned) {
            return SkillResult {
                agent: agent.to_string(),
                path: Some(display_path(&path, home)),
                status: format!("error: {e}"),
            };
        }
    }

    SkillResult {
        agent: agent.to_string(),
        path: Some(display_path(&primary, home)),
        status: "installed".to_string(),
    }
}

/// Return the embedded files and their installation directory.
/// Claude uses `~/.claude/skills/clockwork`. Codex, Gemini CLI, `OpenCode`,
/// and Pi share `~/.agents/skills/clockwork`. Cursor installs per project.
fn skill_target(agent: &str, home: &Path) -> (Vec<(&'static str, &'static str)>, PathBuf) {
    let files = vec![("SKILL.md", SKILL_MD), ("reference.md", REFERENCE_MD)];
    let dir = match agent {
        "claude" => home.join(".claude/skills/clockwork"),
        "codex" | "gemini" | "opencode" | "pi" => home.join(".agents/skills/clockwork"),
        // Report the absolute project path for Cursor's local installation.
        "cursor" => std::env::current_dir()
            .unwrap_or_default()
            .join(".cursor/skills/clockwork"),
        _ => return (vec![], home.to_path_buf()),
    };
    (files, dir)
}

/// Check if an agent binary is available on `PATH`.
fn is_agent_detected(agent: &str) -> bool {
    match agent {
        "claude" | "codex" | "gemini" | "opencode" | "pi" => {
            crate::util::detect::is_binary_on_path(agent)
        }
        "cursor" => cursor_detected(),
        _ => false,
    }
}

/// Cursor doesn't have a single CLI binary to check.
/// Check for the `~/.cursor` directory or the `cursor` binary.
fn cursor_detected() -> bool {
    if let Some(home) = dirs::home_dir() {
        if home.join(".cursor").exists() {
            return true;
        }
    }
    Command::new("which")
        .arg("cursor")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn display_path(path: &Path, home: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(home) {
        format!("~/{}", rel.display())
    } else {
        path.display().to_string()
    }
}

fn print_post_install_note(agent: &str, path: Option<&str>, _home: &Path) {
    // Only Cursor needs a reminder to repeat installation in each project.
    if agent == "cursor" {
        if let Some(p) = path {
            println!();
            println!("Note for Cursor:");
            println!("  Installed into this project at {p}.");
            println!("  Cursor reads skills per project. Run `clockwork setup --agent cursor`");
            println!("  in each project where you want clockwork available.");
        }
    }
}

/// List installed skills and their versions.
fn list_skills(json_output: bool) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;

    let mut entries: Vec<SkillResult> = Vec::new();

    for &agent in KNOWN_AGENTS {
        let (files, dir) = skill_target(agent, &home);
        if files.is_empty() {
            continue;
        }
        let primary = dir.join(files[0].0);
        if primary.exists() {
            let version = read_skill_version(&primary);
            let outdated = version.as_deref() != Some(SKILL_VERSION);
            let status = if outdated {
                format!(
                    "installed (v{}, current: v{SKILL_VERSION})",
                    version.as_deref().unwrap_or("unknown")
                )
            } else {
                format!("installed (v{SKILL_VERSION})")
            };
            entries.push(SkillResult {
                agent: agent.to_string(),
                path: Some(display_path(&primary, &home)),
                status,
            });
        } else {
            entries.push(SkillResult {
                agent: agent.to_string(),
                path: None,
                status: "not installed".to_string(),
            });
        }
    }

    if json_output {
        print_results_json(&entries)?;
    } else {
        println!("{:<12} {:<40} Path", "Agent", "Status");
        for r in &entries {
            println!(
                "{:<12} {:<40} {}",
                r.agent,
                r.status,
                r.path.as_deref().unwrap_or("-")
            );
        }
    }

    Ok(())
}

fn read_skill_version(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let prefix = "<!-- clockwork-skill v";
    let start = content.find(prefix)? + prefix.len();
    let end = content[start..].find(" -->")?;
    Some(content[start..start + end].to_string())
}

struct SkillResult {
    agent: String,
    path: Option<String>,
    status: String,
}
