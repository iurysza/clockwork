use serde::de::Error as _;
use serde::ser::SerializeMap as _;
use serde::{Deserialize, Serialize};

use crate::commands::action_input::{build_webhook_action_with_policy, validate_tags};
use crate::model::action::{Action, HttpMethod};

use super::error::JobError;
use super::name::JobName;

/// Job source containing the schedule and action, but no activation state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobDefinition {
    pub name: JobName,
    /// Cron, recurring duration, or one-time timestamp. The CLI resolves relative times.
    pub schedule: String,
    pub action: JobAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Exactly one action variant after parsing. The source uses a plain mapping:
/// `action: { prompt: { ... } }`.
#[derive(Debug, Clone, PartialEq)]
pub enum JobAction {
    Command(CommandAction),
    Prompt(PromptAction),
    Webhook(WebhookAction),
}

impl Serialize for JobAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Command(action) => map.serialize_entry("command", action)?,
            Self::Prompt(action) => map.serialize_entry("prompt", action)?,
            Self::Webhook(action) => map.serialize_entry("webhook", action)?,
        }
        map.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobActionWire {
    command: Option<CommandAction>,
    prompt: Option<PromptAction>,
    webhook: Option<WebhookAction>,
}

impl<'de> Deserialize<'de> for JobAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = JobActionWire::deserialize(deserializer)?;
        match (wire.command, wire.prompt, wire.webhook) {
            (Some(action), None, None) => Ok(Self::Command(action)),
            (None, Some(action), None) => Ok(Self::Prompt(action)),
            (None, None, Some(action)) => Ok(Self::Webhook(action)),
            _ => Err(D::Error::custom(
                "action must contain exactly one of command, prompt, or webhook",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAction {
    pub command: String,
    #[serde(default)]
    pub shell: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookAction {
    pub url: String,
    #[serde(default)]
    pub method: HttpMethod,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

impl JobAction {
    pub fn kind(&self) -> ActionKind {
        match self {
            Self::Command(_) => ActionKind::Command,
            Self::Prompt(_) => ActionKind::Prompt,
            Self::Webhook(_) => ActionKind::Webhook,
        }
    }

    /// Build a runtime action after checking input lengths and webhook URL policy.
    pub fn to_runtime_action(&self, allow_insecure_http: bool) -> Result<Action, JobError> {
        match self {
            Self::Command(cmd) => {
                if cmd.command.len() > crate::commands::action_input::MAX_COMMAND_LEN {
                    return Err(JobError::invalid_input(format!(
                        "command exceeds maximum length of {} bytes",
                        crate::commands::action_input::MAX_COMMAND_LEN
                    )));
                }
                Ok(Action::Run {
                    command: cmd.command.clone(),
                    shell: cmd.shell,
                    workdir: cmd.workdir.clone(),
                })
            }
            Self::Prompt(prompt) => {
                if prompt.text.len() > crate::commands::action_input::MAX_PROMPT_LEN {
                    return Err(JobError::invalid_input(format!(
                        "prompt exceeds maximum length of {} bytes",
                        crate::commands::action_input::MAX_PROMPT_LEN
                    )));
                }
                Ok(Action::Prompt {
                    text: prompt.text.clone(),
                    agent: prompt.profile.clone(),
                    cwd: prompt.cwd.clone(),
                })
            }
            Self::Webhook(hook) => build_webhook_action_with_policy(
                &hook.url,
                hook.method,
                hook.headers.clone(),
                hook.body.clone(),
                allow_insecure_http,
            )
            .map_err(|e| JobError::invalid_input(e.to_string())),
        }
    }
}

/// Non-secret action classification for previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Command,
    Prompt,
    Webhook,
}

impl std::fmt::Display for ActionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Command => write!(f, "command"),
            Self::Prompt => write!(f, "prompt"),
            Self::Webhook => write!(f, "webhook"),
        }
    }
}

impl JobDefinition {
    /// Validate tags, schedule, and action using the caller's timestamp.
    pub fn validate(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        allow_insecure_http: bool,
    ) -> Result<(), JobError> {
        validate_tags(&self.tags).map_err(|e| JobError::invalid_input(e.to_string()))?;

        // One parser for every path: the same schedule grammar the runtime
        // dispatcher accepts.
        let parsed = crate::schedule::parser::parse_schedule(&self.schedule, now)
            .map_err(|e| JobError::invalid_input(e.to_string()))?;
        let _ = parsed;

        // Builds the runtime action, which enforces webhook URL policy and
        // input limits shared with every other action path.
        self.action
            .to_runtime_action(allow_insecure_http)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_uses_a_plain_single_variant_mapping() {
        let source =
            "name: test\nschedule: every 1h\naction:\n  command:\n    command: echo hello\n";
        let definition: JobDefinition = serde_norway::from_str(source).unwrap();
        assert!(matches!(definition.action, JobAction::Command(_)));

        let serialized = serde_norway::to_string(&definition).unwrap();
        assert!(serialized.contains("action:\n  command:\n    command: echo hello"));
        assert!(!serialized.contains("!command"));
    }

    #[test]
    fn action_rejects_zero_or_multiple_variants() {
        for source in [
            "name: test\nschedule: every 1h\naction: {}\n",
            "name: test\nschedule: every 1h\naction:\n  command:\n    command: true\n  prompt:\n    text: hi\n",
        ] {
            assert!(serde_norway::from_str::<JobDefinition>(source).is_err());
        }
    }
}
