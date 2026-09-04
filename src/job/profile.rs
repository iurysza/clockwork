//! Generic agent-profile resolution for managed prompt jobs.

use std::collections::BTreeMap;

use crate::model::config::AgentProfile;
use crate::store::config;

use super::definition::{JobAction, JobDefinition};
use super::error::JobError;

/// Require a prompt job to resolve to a registered profile and validate its
/// effective working directory. A job-level `cwd` overrides the profile cwd.
pub fn profile_contract(
    definition: &JobDefinition,
    agents: &BTreeMap<String, AgentProfile>,
    default_agent: Option<&str>,
) -> Result<(), JobError> {
    let JobAction::Prompt(prompt) = &definition.action else {
        return Ok(());
    };

    let profile_name = prompt.profile.as_deref().or(default_agent).ok_or_else(|| {
        JobError::invalid_input("prompt jobs need --profile <name> or a configured default agent")
    })?;
    let profile = agents.get(profile_name).ok_or_else(|| {
        JobError::invalid_input(format!(
            "prompt profile '{profile_name}' is not registered. Add it with: clockwork agent add {profile_name} --bin <path>"
        ))
    })?;

    if let Some(cwd) = prompt.cwd.as_deref().or(profile.cwd.as_deref()) {
        crate::util::path::resolve_directory(cwd)
            .map_err(|error| JobError::invalid_input(error.to_string()))?;
    }

    Ok(())
}

/// Complete inspected profile state.
#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    pub agents: BTreeMap<String, AgentProfile>,
    pub default_agent: Option<String>,
}

pub(crate) trait ProfileStore {
    fn snapshot(&self) -> Result<ProfileSnapshot, JobError>;
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
}
