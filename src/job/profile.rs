//! Private agent-profile ownership for managed prompt jobs.
//!
//! A managed prompt job may carry `pi-profile.json` beside its source. The
//! coordinator owns the derived runtime profile `clockwork-pi-<job>`: it
//! upserts the profile after the source write and removes it before the
//! source removal, in the prescribed mutation order. Profile validation
//! reuses the same rules as the Pi launcher.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::model::config::AgentProfile;
use crate::store::config;

use super::definition::{JobAction, JobDefinition};
use super::error::JobError;
use super::name::JobName;

const PROFILE_PREFIX: &str = "clockwork-pi-";
const THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Derived profile name for a managed Pi prompt job.
pub fn managed_profile_name(job: &JobName) -> String {
    format!("{PROFILE_PREFIX}{job}")
}

/// Launcher binary for the derived profile. Mirrors the removed JS wrapper:
/// `$CLOCKWORK_PI_BIN` or `~/.local/bin/clockwork-pi`.
pub fn launcher_bin() -> String {
    if let Ok(bin) = std::env::var("CLOCKWORK_PI_BIN") {
        if !bin.is_empty() {
            return bin;
        }
    }
    dirs::home_dir().map_or_else(
        || "clockwork-pi".to_string(),
        |home| home.join(".local/bin/clockwork-pi").display().to_string(),
    )
}

/// The profile the coordinator installs for a Pi prompt job.
pub fn desired_profile(job: &JobName) -> AgentProfile {
    AgentProfile {
        bin: launcher_bin(),
        args: vec!["--job".to_string(), job.to_string()],
        prompt_stdin: true,
    }
}

/// Parsed `pi-profile.json`. Mirrors `validatePiProfile` in pi-launcher.mjs;
/// the launcher and the planner share one validation policy. The fields are
/// the validated schema surface; production callers only need pass/fail, so
/// they are consumed by the test suite.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct PiProfile {
    pub version: u64,
    pub cwd: String,
    pub model: String,
    pub thinking: String,
    pub tools: Vec<String>,
    #[serde(rename = "approveProjectFiles")]
    pub approve_project_files: bool,
}

const PI_PROFILE_KEYS: &[&str] = &[
    "version",
    "cwd",
    "model",
    "thinking",
    "tools",
    "approveProjectFiles",
];

/// Validate raw `pi-profile.json` content. Fails closed on malformed input.
pub fn validate_pi_profile(raw: &str, require_cwd: bool) -> Result<PiProfile, String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("pi-profile.json is not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "pi-profile.json must contain an object".to_string())?;
    let extra: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !PI_PROFILE_KEYS.contains(key))
        .collect();
    if !extra.is_empty() {
        return Err(format!(
            "pi-profile.json contains unsupported keys: {}",
            extra.join(", ")
        ));
    }

    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "pi-profile.json version must be 1".to_string())?;
    if version != 1 {
        return Err("pi-profile.json version must be 1".to_string());
    }
    let cwd = object
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "pi-profile.json cwd is required".to_string())?;
    let model = object
        .get("model")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "pi-profile.json model must be provider/model".to_string())?;
    if model.split('/').count() != 2
        || model
            .split('/')
            .any(|part| part.is_empty() || part.contains(char::is_whitespace))
    {
        return Err("pi-profile.json model must be provider/model".to_string());
    }
    let thinking = object
        .get("thinking")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "pi-profile.json thinking is invalid".to_string())?;
    if !THINKING_LEVELS.contains(&thinking) {
        return Err("pi-profile.json thinking is invalid".to_string());
    }
    let tools = validate_pi_tools(object)?;
    let approve = object
        .get("approveProjectFiles")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "pi-profile.json approveProjectFiles must be boolean".to_string())?;

    if require_cwd {
        let expanded = if cwd == "~" {
            dirs::home_dir()
                .ok_or_else(|| "pi-profile.json cwd cannot expand ~".to_string())?
                .display()
                .to_string()
        } else if let Some(rest) = cwd.strip_prefix("~/") {
            dirs::home_dir()
                .ok_or_else(|| "pi-profile.json cwd cannot expand ~".to_string())?
                .join(rest)
                .display()
                .to_string()
        } else {
            cwd.to_string()
        };
        if !std::path::Path::new(&expanded).is_dir() {
            return Err(format!(
                "pi-profile.json cwd is not a directory: {expanded}"
            ));
        }
    }

    Ok(PiProfile {
        version,
        cwd: cwd.to_string(),
        model: model.to_string(),
        thinking: thinking.to_string(),
        tools,
        approve_project_files: approve,
    })
}

/// Validate the `tools` array: non-empty, safe lowercase names, no
/// duplicates. Mirrors the launcher policy exactly.
fn validate_pi_tools(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<String>, String> {
    const TOOLS_ERROR: &str = "pi-profile.json tools must be a non-empty safe string array";
    let tools = object
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| TOOLS_ERROR.to_string())?;
    let is_safe = |tool: &str| {
        tool.chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
            && tool
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    };
    if tools.is_empty()
        || tools
            .iter()
            .any(|tool| tool.as_str().is_none_or(|t| !is_safe(t)))
    {
        return Err(TOOLS_ERROR.to_string());
    }
    let mut unique = std::collections::BTreeSet::new();
    if tools
        .iter()
        .any(|tool| !unique.insert(tool.as_str().expect("validated string")))
    {
        return Err("pi-profile.json tools must not contain duplicates".to_string());
    }
    Ok(tools
        .iter()
        .map(|tool| tool.as_str().expect("validated string").to_string())
        .collect())
}

/// Raw `pi-profile.json` content loaded next to a managed source.
#[derive(Debug, Clone)]
pub struct PiProfileSource {
    pub raw: String,
}

/// Fail-closed profile contract for one managed definition.
///
/// - `pi-profile.json` is only allowed for prompt jobs.
/// - A malformed launcher profile rejects the operation.
/// - A prompt job with a launcher profile must reference the derived
///   `clockwork-pi-<job>` profile; the coordinator owns it.
/// - A derived profile that already exists with different settings is an
///   unmanaged collision and rejects the operation.
/// - A prompt job referencing an unregistered profile rejects the operation.
///
/// Returns the derived profile the coordinator must upsert, if any.
pub fn profile_contract(
    definition: &JobDefinition,
    pi_profile: Option<&PiProfileSource>,
    agents: &BTreeMap<String, AgentProfile>,
    default_agent: Option<&str>,
    managed_profile: &AgentProfile,
) -> Result<Option<AgentProfile>, JobError> {
    let job = &definition.name;
    if pi_profile.is_some() {
        if !matches!(definition.action, JobAction::Prompt(_)) {
            return Err(JobError::invalid_input(format!(
                "pi-profile.json is only allowed for prompt jobs; job '{job}' has a {} action",
                definition.action.kind()
            )));
        }
        let expected_name = managed_profile_name(job);
        let JobAction::Prompt(prompt) = &definition.action else {
            unreachable!("action kind checked above");
        };
        if prompt.profile.as_deref() != Some(expected_name.as_str()) {
            return Err(JobError::invalid_input(format!(
                "prompt profile must be '{expected_name}' when the source provides pi-profile.json"
            )));
        }
        let desired = managed_profile.clone();
        if agents
            .get(&expected_name)
            .is_some_and(|existing| *existing != desired)
        {
            return Err(JobError::invalid_input(format!(
                "agent profile '{expected_name}' already exists with different settings and is not owned by job '{job}'"
            )));
        }
        return Ok(Some(desired));
    }

    if let JobAction::Prompt(prompt) = &definition.action {
        let profile = prompt.profile.as_deref().or(default_agent).ok_or_else(|| {
            JobError::invalid_input(
                "prompt jobs need --profile <name> or a configured default agent",
            )
        })?;
        if !agents.contains_key(profile) {
            return Err(JobError::invalid_input(format!(
                "prompt profile '{profile}' is not registered. Add it with: clockwork agent add {profile} --bin <path>"
            )));
        }
    }
    Ok(None)
}

/// Complete inspected profile state.
#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    pub agents: BTreeMap<String, AgentProfile>,
    pub default_agent: Option<String>,
}

/// Profile mutations the coordinator may apply. Removal is idempotent so
/// an interrupted delete can be rerun safely.
pub(crate) enum ProfileMutation {
    Upsert { name: String, profile: AgentProfile },
    Remove { name: String },
}

/// Private profile adapter over the application config.
pub(crate) trait ProfileStore {
    fn snapshot(&self) -> Result<ProfileSnapshot, JobError>;
    fn apply(&self, mutation: ProfileMutation) -> Result<(), JobError>;
}

pub struct FsProfileStore;

impl ProfileStore for FsProfileStore {
    fn snapshot(&self) -> Result<ProfileSnapshot, JobError> {
        let config = config::load_config().map_err(|error| JobError::RuntimeFailure {
            message: format!("{error:#}"),
        })?;
        Ok(ProfileSnapshot {
            agents: config.agents,
            default_agent: config.default_agent,
        })
    }

    fn apply(&self, mutation: ProfileMutation) -> Result<(), JobError> {
        config::update_config(|config| {
            match mutation {
                ProfileMutation::Upsert { name, profile } => {
                    config.agents.insert(name, profile);
                }
                ProfileMutation::Remove { name } => {
                    if config.agents.remove(&name).is_some()
                        && config.default_agent.as_deref() == Some(name.as_str())
                    {
                        config.default_agent = None;
                    }
                }
            }
            Ok(())
        })
        .map_err(|error| JobError::RuntimeFailure {
            message: format!("{error:#}"),
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "version": 1,
        "cwd": "/tmp",
        "model": "anthropic/claude-sonnet-4",
        "thinking": "low",
        "tools": ["read", "bash-tool"],
        "approveProjectFiles": false
    }"#;

    #[test]
    fn parses_valid_profile() {
        let profile = validate_pi_profile(VALID, false).expect("valid profile");
        assert_eq!(profile.version, 1);
        assert_eq!(profile.cwd, "/tmp");
        assert_eq!(profile.model, "anthropic/claude-sonnet-4");
        assert_eq!(profile.thinking, "low");
        assert_eq!(profile.tools, vec!["read", "bash-tool"]);
        assert!(!profile.approve_project_files);
    }

    #[test]
    fn rejects_malformed_profiles() {
        for (name, raw) in [
            ("bad json", "{not json"),
            ("not object", "[1,2]"),
            (
                "unsupported key",
                r#"{"version":1,"cwd":"/tmp","model":"a/b","thinking":"low","tools":["read"],"approveProjectFiles":false,"extra":true}"#,
            ),
            (
                "bad version",
                r#"{"version":2,"cwd":"/tmp","model":"a/b","thinking":"low","tools":["read"],"approveProjectFiles":false}"#,
            ),
            (
                "bad model",
                r#"{"version":1,"cwd":"/tmp","model":"nomodel","thinking":"low","tools":["read"],"approveProjectFiles":false}"#,
            ),
            (
                "bad thinking",
                r#"{"version":1,"cwd":"/tmp","model":"a/b","thinking":"huge","tools":["read"],"approveProjectFiles":false}"#,
            ),
            (
                "empty tools",
                r#"{"version":1,"cwd":"/tmp","model":"a/b","thinking":"low","tools":[],"approveProjectFiles":false}"#,
            ),
            (
                "duplicate tools",
                r#"{"version":1,"cwd":"/tmp","model":"a/b","thinking":"low","tools":["read","read"],"approveProjectFiles":false}"#,
            ),
            (
                "unsafe tool",
                r#"{"version":1,"cwd":"/tmp","model":"a/b","thinking":"low","tools":["Bad Tool"],"approveProjectFiles":false}"#,
            ),
            (
                "missing cwd",
                r#"{"version":1,"model":"a/b","thinking":"low","tools":["read"],"approveProjectFiles":false}"#,
            ),
        ] {
            assert!(
                validate_pi_profile(raw, false).is_err(),
                "{name} should be rejected"
            );
        }
    }

    #[test]
    fn requires_cwd_to_exist_when_requested() {
        assert!(validate_pi_profile(VALID, true).is_ok());
        let missing = VALID.replace("/tmp", "/definitely/not/here-clockwork");
        assert!(validate_pi_profile(&missing, true).is_err());
    }
}
