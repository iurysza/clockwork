use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::model::config::AgentProfile;
use crate::model::job::JobStatus;

use super::definition::JobAction;
use crate::model::schedule::JobSchedule;
use crate::schedule::occurrence::{latest_due, next_after};
use crate::store::config::load_config;

use super::definition::{ActionKind, JobDefinition};
use super::error::{JobError, NotFound};
use super::name::JobName;
use super::profile::{ProfileStore, profile_contract};
use super::runtime::{RuntimeStore, VersionedRuntimeJob, runtime_revision};
use super::source::{FsSourceStore, SourceStore, VersionedJobSource};
use super::state::{Activation, JobView, ManagedJobState, StateRevision};

/// Complete inspected state for one job: managed source, runtime job,
/// agent profile config, and the derived public state.
#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub name: JobName,
    pub source: Option<VersionedJobSource>,
    pub runtime: Option<VersionedRuntimeJob>,
    pub agents: BTreeMap<String, AgentProfile>,
    pub default_agent: Option<String>,
    pub default_timeout_seconds: u64,
    pub allow_insecure_http: bool,
}

impl JobSnapshot {
    pub fn revision(&self) -> StateRevision {
        StateRevision {
            source: self.source.as_ref().map(|s| s.revision.clone()),
            runtime: self.runtime.as_ref().map(|r| runtime_revision(&r.job)),
            profile: self.profile_revision(),
        }
    }

    /// Pin the resolved generic profile state. A profile change after preview
    /// must move the optimistic revision.
    fn profile_revision(&self) -> Option<String> {
        self.definition()
            .and_then(|definition| self.profile_revision_for(definition))
    }

    pub fn revision_for_definition(&self, definition: &JobDefinition) -> StateRevision {
        StateRevision {
            source: self.source.as_ref().map(|source| source.revision.clone()),
            runtime: self
                .runtime
                .as_ref()
                .map(|runtime| runtime_revision(&runtime.job)),
            profile: self.profile_revision_for(definition),
        }
    }

    fn profile_revision_for(&self, definition: &JobDefinition) -> Option<String> {
        let JobAction::Prompt(prompt) = &definition.action else {
            return None;
        };
        let Some(name) = prompt
            .profile
            .clone()
            .or_else(|| self.default_agent.clone())
        else {
            return Some("rev_profile_unresolved".to_string());
        };
        let bytes = serde_json::to_vec(&(&name, self.agents.get(&name)))
            .expect("agent profiles are JSON serializable");
        Some(super::state::content_revision(&bytes))
    }

    pub fn definition(&self) -> Option<&JobDefinition> {
        self.source.as_ref().map(|s| &s.definition)
    }

    pub fn activation(&self) -> Option<Activation> {
        self.runtime.as_ref().map(|r| match r.job.status {
            JobStatus::Active => Activation::Enabled,
            JobStatus::Paused | JobStatus::Completed | JobStatus::Archived => Activation::Disabled,
        })
    }

    /// Confirm that executable runtime fields still match the managed source
    /// and that the action resolves to a valid profile. Operational fields
    /// such as activation, claims, counters, and history may differ.
    pub fn validate_runtime_definition(&self, _now: DateTime<Utc>) -> Result<(), JobError> {
        let source = self.source.as_ref().ok_or_else(|| {
            JobError::integrity(
                Some(self.name.clone()),
                "a runtime job exists without its managed source",
            )
        })?;
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            JobError::integrity(
                Some(self.name.clone()),
                "managed source has no installed runtime definition",
            )
        })?;
        let job = &runtime.job;

        if job.managed_by.as_deref() != Some("managed-job")
            || job.id != self.name.as_str()
            || job.name.as_deref() != Some(self.name.as_str())
            || job.source_revision.as_deref() != Some(source.revision.as_str())
        {
            return Err(JobError::integrity(
                Some(self.name.clone()),
                "runtime ownership or source revision does not match the managed source",
            ));
        }

        source
            .definition
            .validate(job.created_at, self.allow_insecure_http)
            .map_err(|error| {
                JobError::integrity(
                    Some(self.name.clone()),
                    format!("managed source is no longer valid: {error}"),
                )
            })?;
        let schedule =
            crate::schedule::parser::parse_schedule(&source.definition.schedule, job.created_at)
                .map_err(|error| {
                    JobError::integrity(
                        Some(self.name.clone()),
                        format!("managed schedule is invalid: {error}"),
                    )
                })?
                .to_job_schedule();
        let action = source
            .definition
            .action
            .to_runtime_action(self.allow_insecure_http)
            .map_err(|error| {
                JobError::integrity(
                    Some(self.name.clone()),
                    format!("managed action is invalid: {error}"),
                )
            })?;
        if job.schedule_input != source.definition.schedule
            || job.schedule != schedule
            || job.action != action
            || source
                .definition
                .timeout
                .is_some_and(|timeout| job.timeout_seconds != timeout)
            || job.tags != source.definition.tags
            || job.skip_remaining != 0
            || job.on_failure.is_some()
            || job.on_failure_shell
        {
            return Err(JobError::integrity(
                Some(self.name.clone()),
                "runtime definition was changed outside clockwork job commands",
            ));
        }
        Ok(())
    }

    fn validate_executable_integrity(&self, now: DateTime<Utc>) -> Result<(), JobError> {
        self.validate_runtime_definition(now)?;
        let source = self
            .source
            .as_ref()
            .expect("runtime validation requires a source");
        profile_contract(
            &source.definition,
            &self.agents,
            self.default_agent.as_deref(),
        )
        .map_err(|error| {
            JobError::integrity(
                Some(self.name.clone()),
                format!("agent profile state is invalid: {error}"),
            )
        })
    }

    /// Derive the public managed state, fail closed on contradiction.
    pub fn derive_state(&self, now: DateTime<Utc>) -> Result<ManagedJobState, JobError> {
        self.derive_state_checked(now, true)
    }

    /// Derive lifecycle state for a planned repair.
    pub(crate) fn derive_plannable_state(
        &self,
        now: DateTime<Utc>,
    ) -> Result<ManagedJobState, JobError> {
        self.derive_state_checked(now, false)
    }

    fn derive_state_checked(
        &self,
        now: DateTime<Utc>,
        require_installed_profile: bool,
    ) -> Result<ManagedJobState, JobError> {
        let Some(source) = &self.source else {
            return if self.runtime.is_some() {
                Err(JobError::integrity(
                    Some(self.name.clone()),
                    "a runtime job exists without its managed source",
                ))
            } else {
                Err(JobError::JobNotFound(NotFound(self.name.clone())))
            };
        };
        let Some(runtime) = &self.runtime else {
            return Ok(ManagedJobState::Draft {
                source_revision: source.revision.clone(),
            });
        };
        let job = &runtime.job;
        if require_installed_profile {
            self.validate_executable_integrity(now)?;
        } else {
            self.validate_runtime_definition(now)?;
        }

        if job.status == JobStatus::Archived {
            return Err(JobError::integrity(
                Some(self.name.clone()),
                "runtime job is archived; archived jobs are outside the managed lifecycle",
            ));
        }

        let base = |next: ManagedJobState| Ok(next);
        let generation = job.generation;
        let revision = source.revision.clone();

        if let Some(claim) = &job.in_flight {
            return base(ManagedJobState::Running {
                source_revision: revision,
                runtime_generation: generation,
                run_id: claim.run_id.clone(),
                scheduled_for: claim.scheduled_for,
            });
        }

        match job.status {
            JobStatus::Paused => base(ManagedJobState::Disabled {
                source_revision: revision,
                runtime_generation: generation,
            }),
            JobStatus::Completed => base(ManagedJobState::Completed {
                source_revision: revision,
                runtime_generation: generation,
                last_run: job.last_run.clone(),
            }),
            JobStatus::Archived => unreachable!("archived handled above"),
            JobStatus::Active => match &job.schedule {
                JobSchedule::OneShot { fire_at } => {
                    // A one-shot whose fire time passed without a claim
                    // or completion is contradictory stored data.
                    if fire_at <= &now {
                        return Err(JobError::integrity(
                            Some(self.name.clone()),
                            "one-shot fire time is in the past but the run was neither claimed nor completed",
                        ));
                    }
                    base(ManagedJobState::Scheduled {
                        source_revision: revision,
                        runtime_generation: generation,
                        next_run: *fire_at,
                    })
                }
                JobSchedule::RecurringCron { .. } | JobSchedule::RecurringInterval { .. } => {
                    let anchor = job.last_scheduled_at.unwrap_or(job.created_at);
                    let latest = latest_due(&job.schedule, anchor, now).map_err(|error| {
                        JobError::integrity(
                            Some(self.name.clone()),
                            format!("stored schedule is invalid: {error}"),
                        )
                    })?;
                    let next_anchor = latest.unwrap_or(anchor);
                    let next_run = next_after(&job.schedule, next_anchor)
                        .map_err(|error| {
                            JobError::integrity(
                                Some(self.name.clone()),
                                format!("stored schedule is invalid: {error}"),
                            )
                        })?
                        .filter(|next| *next > now)
                        .ok_or_else(|| {
                            JobError::integrity(
                                Some(self.name.clone()),
                                "recurring schedule has no future occurrence",
                            )
                        })?;
                    base(ManagedJobState::Scheduled {
                        source_revision: revision,
                        runtime_generation: generation,
                        next_run,
                    })
                }
            },
        }
    }
}

/// Imperative shell: reads sources, runtime state, and profile config and
/// assembles snapshots. No policy lives here — the planner owns that.
pub struct StateInspector {
    sources: FsSourceStore,
    runtime: super::runtime::FsRuntimeStore,
    profiles: super::profile::FsProfileStore,
}

impl Default for StateInspector {
    fn default() -> Self {
        Self::new()
    }
}

impl StateInspector {
    pub fn new() -> Self {
        Self {
            sources: FsSourceStore,
            runtime: super::runtime::FsRuntimeStore,
            profiles: super::profile::FsProfileStore,
        }
    }

    pub fn verify_managed_runtime(
        &self,
        job: &crate::model::job::Job,
        now: DateTime<Utc>,
    ) -> Result<(), JobError> {
        if job.managed_by.as_deref() != Some("managed-job") {
            return Ok(());
        }
        let identity = job.name.as_deref().unwrap_or(&job.id);
        let name = JobName::parse(identity).map_err(|error| {
            JobError::integrity(
                None,
                format!("managed runtime identity is invalid: {error}"),
            )
        })?;
        self.snapshot(&name)?.validate_executable_integrity(now)
    }

    pub fn snapshot(&self, name: &JobName) -> Result<JobSnapshot, JobError> {
        let config = load_config().map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;
        let profiles = self.profiles.snapshot()?;
        let source = self.sources.load(name)?;
        Ok(JobSnapshot {
            name: name.clone(),
            source,
            runtime: self.runtime.snapshot(name)?,
            agents: profiles.agents.clone(),
            default_agent: profiles.default_agent.clone(),
            default_timeout_seconds: config.default_timeout_seconds,
            allow_insecure_http: config.allow_insecure_http,
        })
    }

    /// All managed jobs: the union of source directories and runtime jobs.
    pub fn list(&self) -> Result<Vec<(JobName, JobSnapshot)>, JobError> {
        let config = load_config().map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;
        let profiles = self.profiles.snapshot()?;
        let mut names: std::collections::BTreeSet<JobName> =
            FsSourceStore::names()?.into_iter().collect();

        let state = crate::store::state::load_state().map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;
        for (id, job) in &state.jobs {
            if let Ok(name) = JobName::parse(id) {
                names.insert(name);
            } else if let Some(name) = job.name.as_deref().and_then(|n| JobName::parse(n).ok()) {
                names.insert(name);
            }
        }

        names
            .into_iter()
            .map(|name| {
                let source = self.sources.load(&name)?;
                let snapshot = JobSnapshot {
                    name: name.clone(),
                    source,
                    runtime: self.runtime.snapshot(&name)?,
                    agents: profiles.agents.clone(),
                    default_agent: profiles.default_agent.clone(),
                    default_timeout_seconds: config.default_timeout_seconds,
                    allow_insecure_http: config.allow_insecure_http,
                };
                Ok((name, snapshot))
            })
            .collect()
    }

    pub fn view(&self, name: &JobName, now: DateTime<Utc>) -> Result<JobView, JobError> {
        let snapshot = self.snapshot(name)?;
        let state = snapshot.derive_state(now)?;
        let (schedule_input, action_kind, tags) = snapshot
            .definition()
            .map_or((String::new(), ActionKind::Command, Vec::new()), |d| {
                (d.schedule.clone(), d.action.kind(), d.tags.clone())
            });
        Ok(JobView {
            name: name.clone(),
            activation: snapshot.activation().unwrap_or(Activation::Disabled),
            revision: snapshot.revision(),
            schedule_input,
            action_kind,
            tags,
            state,
        })
    }
}
