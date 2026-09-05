use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::schedule::JobSchedule;
use crate::schedule::occurrence::next_after;
use crate::schedule::parser::parse_schedule;

use super::definition::{ActionKind, JobDefinition};
use super::error::{JobError, NotFound};
use super::inspect::JobSnapshot;
use super::name::JobName;
use super::profile::profile_contract;
use super::state::{ManagedJobState, StateRevision};

/// Job operations requested by the CLI, separate from storage mutations.
#[derive(Debug, Clone)]
pub enum JobOperation {
    Create(CreateJob),
    Update(UpdateJob),
    Enable(JobName),
    Disable(JobName),
    Delete(JobName),
    Trigger(JobName),
}

impl JobOperation {
    pub fn name(&self) -> &JobName {
        match self {
            Self::Create(create) => &create.definition.name,
            Self::Update(update) => &update.name,
            Self::Enable(name) | Self::Disable(name) | Self::Delete(name) | Self::Trigger(name) => {
                name
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateJob {
    pub definition: JobDefinition,
}

#[derive(Debug, Clone)]
pub struct UpdateJob {
    pub name: JobName,
    pub definition: JobDefinition,
}

/// One ordered step of a planned mutation. The coordinator applies them in
/// sequence; each step is individually idempotent so an interrupted command
/// can be rerun safely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    WriteSource,
    CreateRuntimeDisabled,
    UpdateRuntimeDefinition,
    DisableScheduling,
    EnableScheduling,
    ReplaceRuntimeGeneration,
    RemoveRuntime,
    RemoveSource,
    TriggerRun,
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::WriteSource => "write the managed source",
            Self::CreateRuntimeDisabled => "install the runtime job as disabled",
            Self::UpdateRuntimeDefinition => "update the runtime definition",
            Self::DisableScheduling => "disable scheduling",
            Self::EnableScheduling => "restore scheduling",
            Self::ReplaceRuntimeGeneration => {
                "replace the runtime generation (new disabled generation)"
            }
            Self::RemoveRuntime => "remove the runtime job",
            Self::RemoveSource => "remove the managed source",
            Self::TriggerRun => "run the action now",
        };
        f.write_str(text)
    }
}

/// Whether a plan permits future runs or starts an action immediately.
/// Previews include the action type but omit commands, prompts, and webhook data.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalEffect {
    None,
    FutureSchedule {
        next_run: DateTime<Utc>,
        action: ActionKind,
    },
    ImmediateTrigger {
        action: ActionKind,
    },
}

/// Runtime definition prepared by the planner using the caller's timestamp.
#[derive(Debug, Clone)]
pub struct PlannedRuntimeDefinition {
    pub schedule_input: String,
    pub schedule: JobSchedule,
    pub action: crate::model::action::Action,
    pub timeout_seconds: u64,
    pub tags: Vec<String>,
    pub source_revision: String,
}

/// The validated, ordered result of planning. `expected_state` is the
/// postcondition the coordinator must verify after mutation; it is `None`
/// only for delete, where the postcondition is absence.
#[derive(Debug, Clone)]
pub struct PlannedChange {
    pub revision: StateRevision,
    pub operation: &'static str,
    pub job: JobName,
    pub current_state: ManagedJobState,
    pub expected_state: Option<ManagedJobState>,
    pub changes: Vec<Change>,
    pub external_effect: ExternalEffect,
    /// Definition payload for operations that write the source.
    pub definition: Option<JobDefinition>,
    /// Next run the runtime would claim, when scheduling is active.
    pub next_run: Option<DateTime<Utc>>,
    /// Private runtime payload for the coordinator (not part of previews).
    pub(crate) runtime: Option<PlannedRuntimeDefinition>,
}

impl PlannedChange {
    /// True when applying this plan would not touch the store.
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Plan an operation from a snapshot and the caller's timestamp.
/// Returns ordered changes or a typed error without reading or writing state.
pub struct JobPlanner;

impl JobPlanner {
    pub fn plan(
        operation: &JobOperation,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let mut planned = match operation {
            JobOperation::Create(create) => Self::plan_create(create, snapshot, now),
            JobOperation::Update(update) => Self::plan_update(update, snapshot, now),
            JobOperation::Enable(name) => Self::plan_enable(name, snapshot, now),
            JobOperation::Disable(name) => Self::plan_disable(name, snapshot, now),
            JobOperation::Delete(name) => Self::plan_delete(name, snapshot, now),
            JobOperation::Trigger(name) => Self::plan_trigger(name, snapshot, now),
        }?;
        planned.revision = match operation {
            JobOperation::Create(create) => snapshot.revision_for_definition(&create.definition),
            JobOperation::Update(update) => snapshot.revision_for_definition(&update.definition),
            JobOperation::Enable(_)
            | JobOperation::Disable(_)
            | JobOperation::Delete(_)
            | JobOperation::Trigger(_) => snapshot.revision(),
        };
        Ok(planned)
    }

    fn plan_create(
        create: &CreateJob,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let definition = &create.definition;
        let name = &definition.name;
        let runtime = Self::validate_definition(definition, snapshot, now)?;

        if let Some(existing) = &snapshot.source {
            if existing.definition == *definition {
                // Repeated create with the same definition completes an
                // interrupted runtime install, otherwise it is a no-op.
                let current = snapshot.derive_plannable_state(now)?;
                let mut changes = Vec::new();
                let expected = if snapshot.runtime.is_some() {
                    current.clone()
                } else {
                    changes.push(Change::CreateRuntimeDisabled);
                    ManagedJobState::Disabled {
                        source_revision: current.source_revision().to_string(),
                        runtime_generation: 0,
                    }
                };
                return Ok(PlannedChange {
                    revision: snapshot.revision(),
                    operation: "create",
                    job: name.clone(),
                    current_state: current.clone(),
                    expected_state: Some(expected),
                    changes,
                    external_effect: ExternalEffect::None,
                    definition: None,
                    next_run: None,
                    runtime: Some(runtime),
                });
            }
            return Err(JobError::JobAlreadyExists(name.clone()));
        }
        if snapshot.runtime.is_some() {
            return Err(JobError::integrity(
                Some(name.clone()),
                "a runtime job with this name exists without a managed source",
            ));
        }

        let source_revision = runtime.source_revision.clone();
        let changes = vec![Change::WriteSource, Change::CreateRuntimeDisabled];

        Ok(PlannedChange {
            revision: snapshot.revision(),
            operation: "create",
            job: name.clone(),
            current_state: ManagedJobState::Draft {
                source_revision: source_revision.clone(),
            },
            expected_state: Some(ManagedJobState::Disabled {
                source_revision,
                runtime_generation: 0,
            }),
            changes,
            external_effect: ExternalEffect::None,
            definition: Some(definition.clone()),
            next_run: None,
            runtime: Some(runtime),
        })
    }

    /// Resume an interrupted update: the source already holds the new
    /// definition and the runtime was left disabled behind it. Reapplying
    /// the runtime definition completes the operation.
    fn plan_interrupted_resume(
        update: &UpdateJob,
        existing: &super::source::VersionedJobSource,
        snapshot: &JobSnapshot,
        runtime: &PlannedRuntimeDefinition,
    ) -> Option<PlannedChange> {
        let name = &update.name;
        let interrupted = existing.definition == update.definition
            && snapshot.runtime.as_ref().is_some_and(|runtime| {
                runtime.job.source_revision.as_deref() != Some(existing.revision.as_str())
                    && runtime.job.status == crate::model::job::JobStatus::Paused
                    && runtime.job.in_flight.is_none()
                    && runtime.job.managed_by.as_deref() == Some("managed-job")
            });
        if !interrupted {
            return None;
        }
        let generation = snapshot
            .runtime
            .as_ref()
            .map_or(0, |runtime| runtime.job.generation);
        let current = ManagedJobState::Disabled {
            source_revision: existing.revision.clone(),
            runtime_generation: generation,
        };
        let changes = vec![Change::UpdateRuntimeDefinition];
        Some(PlannedChange {
            revision: snapshot.revision(),
            operation: "update",
            job: name.clone(),
            current_state: current.clone(),
            expected_state: Some(current),
            changes,
            external_effect: ExternalEffect::None,
            definition: None,
            next_run: None,
            runtime: Some(runtime.clone()),
        })
    }

    /// Assemble the ordered update steps and expected postcondition. Enabled
    /// jobs are disabled first and restored last, so interruption stays safe.
    fn plan_update_changes(
        update: &UpdateJob,
        runtime: &PlannedRuntimeDefinition,
        existing: &super::source::VersionedJobSource,
        snapshot: &JobSnapshot,
        current: &ManagedJobState,
        now: DateTime<Utc>,
    ) -> Result<(Vec<Change>, Option<ManagedJobState>), JobError> {
        let source_revision = runtime.source_revision.clone();
        let generation = snapshot.runtime.as_ref().map_or(0, |r| r.job.generation);
        let completed_schedule_change = matches!(current, ManagedJobState::Completed { .. })
            && existing.definition.schedule != update.definition.schedule;

        if matches!(current, ManagedJobState::Scheduled { .. }) {
            let next_run = next_after(&runtime.schedule, now)
                .map_err(|e| {
                    JobError::invalid_input(format!(
                        "updated schedule has no future occurrence: {e}"
                    ))
                })?
                .ok_or_else(|| {
                    JobError::invalid_input("updated schedule has no future occurrence")
                })?;
            return Ok((
                vec![
                    Change::DisableScheduling,
                    Change::WriteSource,
                    Change::UpdateRuntimeDefinition,
                    Change::EnableScheduling,
                ],
                Some(ManagedJobState::Scheduled {
                    source_revision,
                    runtime_generation: generation,
                    next_run,
                }),
            ));
        }

        if completed_schedule_change {
            return Ok((
                vec![Change::WriteSource, Change::ReplaceRuntimeGeneration],
                Some(ManagedJobState::Disabled {
                    source_revision,
                    runtime_generation: generation.saturating_add(1),
                }),
            ));
        }

        if let ManagedJobState::Completed { last_run, .. } = current {
            return Ok((
                vec![Change::WriteSource, Change::UpdateRuntimeDefinition],
                Some(ManagedJobState::Completed {
                    source_revision,
                    runtime_generation: generation,
                    last_run: last_run.clone(),
                }),
            ));
        }

        if snapshot.runtime.is_none() {
            return Ok((
                vec![Change::WriteSource],
                Some(ManagedJobState::Draft { source_revision }),
            ));
        }

        Ok((
            vec![Change::WriteSource, Change::UpdateRuntimeDefinition],
            Some(ManagedJobState::Disabled {
                source_revision,
                runtime_generation: generation,
            }),
        ))
    }

    fn plan_update(
        update: &UpdateJob,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let name = &update.name;
        let Some(existing) = &snapshot.source else {
            return Err(JobError::JobNotFound(NotFound(name.clone())));
        };
        if update.definition.name != *name {
            return Err(JobError::invalid_input(format!(
                "update cannot rename the job; source name is '{}'",
                update.definition.name
            )));
        }

        if let Some(claim) = snapshot
            .runtime
            .as_ref()
            .and_then(|r| r.job.in_flight.clone())
        {
            return Err(JobError::RunInFlight {
                job: name.clone(),
                run_id: claim.run_id,
                scheduled_for: claim.scheduled_for,
            });
        }

        let runtime = Self::validate_definition(&update.definition, snapshot, now)?;

        // A crash after writing the new source but before updating runtime
        // leaves the prior runtime disabled. It is the intended safe retry
        // state, not arbitrary drift. Only the same complete update can
        // resume it, and it remains disabled after repair.
        if let Some(resume) = Self::plan_interrupted_resume(update, existing, snapshot, &runtime) {
            return Ok(resume);
        }

        let current = snapshot.derive_plannable_state(now)?;
        if existing.definition == update.definition {
            return Ok(PlannedChange {
                revision: snapshot.revision(),
                operation: "update",
                job: name.clone(),
                current_state: current.clone(),
                expected_state: Some(current),
                changes: Vec::new(),
                external_effect: ExternalEffect::None,
                definition: None,
                next_run: None,
                runtime: Some(runtime),
            });
        }

        // Action type changes are rejected; delete and recreate instead.
        if existing.definition.action.kind() != update.definition.action.kind() {
            return Err(JobError::illegal_transition(
                name.clone(),
                current.label(),
                "update",
                "the action type cannot change on update; delete the job and create it again",
                Some(format!("clockwork job delete {name}")),
            ));
        }

        let (changes, expected_state) =
            Self::plan_update_changes(update, &runtime, existing, snapshot, &current, now)?;
        let next_run = expected_state.as_ref().and_then(ManagedJobState::next_run);
        Ok(PlannedChange {
            revision: snapshot.revision(),
            operation: "update",
            job: name.clone(),
            current_state: current.clone(),
            expected_state,
            changes,
            external_effect: ExternalEffect::None,
            definition: Some(update.definition.clone()),
            next_run,
            runtime: Some(runtime),
        })
    }

    /// An already scheduled job is an idempotent enable.
    fn plan_scheduled_enable(
        name: &JobName,
        source: &super::source::VersionedJobSource,
        current: &ManagedJobState,
        snapshot: &JobSnapshot,
    ) -> Option<PlannedChange> {
        let ManagedJobState::Scheduled { next_run, .. } = current else {
            return None;
        };
        let next_run = *next_run;
        Some(PlannedChange {
            revision: snapshot.revision(),
            operation: "enable",
            job: name.clone(),
            current_state: current.clone(),
            expected_state: Some(current.clone()),
            changes: Vec::new(),
            external_effect: ExternalEffect::FutureSchedule {
                next_run,
                action: source.definition.action.kind(),
            },
            definition: None,
            next_run: Some(next_run),
            runtime: None,
        })
    }

    fn plan_enable(
        name: &JobName,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let Some(source) = &snapshot.source else {
            return Err(JobError::JobNotFound(NotFound(name.clone())));
        };
        let current = snapshot.derive_plannable_state(now)?;

        match &current {
            ManagedJobState::Running { run_id, .. } => {
                let scheduled_for = snapshot
                    .runtime
                    .as_ref()
                    .and_then(|r| r.job.in_flight.clone())
                    .map_or(now, |c| c.scheduled_for);
                return Err(JobError::RunInFlight {
                    job: name.clone(),
                    run_id: run_id.clone(),
                    scheduled_for,
                });
            }
            ManagedJobState::Completed { .. } => {
                return Err(JobError::illegal_transition(
                    name.clone(),
                    "completed",
                    "enable",
                    "a completed one-time job needs a new future schedule before enablement",
                    Some(format!(
                        "clockwork job update {name} --schedule <future-time>"
                    )),
                ));
            }
            _ => {}
        }

        let definition = &source.definition;
        let runtime = Self::validate_definition(definition, snapshot, now)?;
        if let Some(scheduled) = Self::plan_scheduled_enable(name, source, &current, snapshot) {
            return Ok(scheduled);
        }
        let next_run = next_after(&runtime.schedule, now)
            .map_err(|e| {
                JobError::illegal_transition(
                    name.clone(),
                    current.label(),
                    "enable",
                    format!("the schedule does not resolve to a future next run: {e}"),
                    Some(format!(
                        "clockwork job update {name} --schedule <future-time>"
                    )),
                )
            })?
            .ok_or_else(|| {
                JobError::illegal_transition(
                    name.clone(),
                    current.label(),
                    "enable",
                    "the schedule does not resolve to a future next run",
                    Some(format!(
                        "clockwork job update {name} --schedule <future-time>"
                    )),
                )
            })?;
        let source_revision = runtime.source_revision.clone();
        let action = definition.action.kind();

        // Draft (interrupted create): install the disabled runtime job first,
        // then enable. Interruption still cannot run the job.
        let mut changes = Vec::new();
        if snapshot.runtime.is_none() {
            changes.push(Change::CreateRuntimeDisabled);
        }
        changes.push(Change::EnableScheduling);

        Ok(PlannedChange {
            revision: snapshot.revision(),
            operation: "enable",
            job: name.clone(),
            current_state: current.clone(),
            expected_state: Some(ManagedJobState::Scheduled {
                source_revision,
                runtime_generation: snapshot.runtime.as_ref().map_or(0, |r| r.job.generation),
                next_run,
            }),
            changes,
            external_effect: ExternalEffect::FutureSchedule { next_run, action },
            definition: None,
            next_run: Some(next_run),
            runtime: Some(runtime),
        })
    }

    fn plan_disable(
        name: &JobName,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let Some(_source) = &snapshot.source else {
            return Err(JobError::JobNotFound(NotFound(name.clone())));
        };
        let current = snapshot.derive_plannable_state(now)?;

        let (changes, expected) = match &current {
            ManagedJobState::Scheduled { .. } => (
                vec![Change::DisableScheduling],
                Some(ManagedJobState::Disabled {
                    source_revision: current.source_revision().to_string(),
                    runtime_generation: snapshot.runtime.as_ref().map_or(0, |r| r.job.generation),
                }),
            ),
            ManagedJobState::Running { .. } => {
                // Disabling prevents future claims; the current run can
                // still finish. The public state stays Running until it does.
                (vec![Change::DisableScheduling], Some(current.clone()))
            }
            // Disabled, Draft, Completed: idempotent no-op.
            _ => (Vec::new(), Some(current.clone())),
        };

        Ok(PlannedChange {
            revision: snapshot.revision(),
            operation: "disable",
            job: name.clone(),
            current_state: current.clone(),
            expected_state: expected.clone(),
            changes,
            external_effect: ExternalEffect::None,
            definition: None,
            next_run: expected
                .as_ref()
                .and_then(super::state::ManagedJobState::next_run),
            runtime: None,
        })
    }

    fn plan_delete(
        name: &JobName,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let Some(_source) = &snapshot.source else {
            return Err(JobError::JobNotFound(NotFound(name.clone())));
        };
        let current = snapshot.derive_plannable_state(now)?;

        if let ManagedJobState::Running { run_id, .. } = &current {
            let scheduled_for = snapshot
                .runtime
                .as_ref()
                .and_then(|r| r.job.in_flight.clone())
                .map_or(now, |c| c.scheduled_for);
            return Err(JobError::RunInFlight {
                job: name.clone(),
                run_id: run_id.clone(),
                scheduled_for,
            });
        }

        let mut changes = Vec::new();
        if matches!(current, ManagedJobState::Scheduled { .. }) {
            changes.push(Change::DisableScheduling);
        }
        if snapshot.runtime.is_some() {
            changes.push(Change::RemoveRuntime);
        }
        changes.push(Change::RemoveSource);

        Ok(PlannedChange {
            revision: snapshot.revision(),
            operation: "delete",
            job: name.clone(),
            current_state: current,
            expected_state: None,
            changes,
            external_effect: ExternalEffect::None,
            definition: None,
            next_run: None,
            runtime: None,
        })
    }

    fn plan_trigger(
        name: &JobName,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let Some(source) = &snapshot.source else {
            return Err(JobError::JobNotFound(NotFound(name.clone())));
        };
        profile_contract(
            &source.definition,
            &snapshot.agents,
            snapshot.default_agent.as_deref(),
        )?;
        let current = snapshot.derive_plannable_state(now)?;

        match &current {
            ManagedJobState::Scheduled { .. } => {}
            ManagedJobState::Running { run_id, .. } => {
                let scheduled_for = snapshot
                    .runtime
                    .as_ref()
                    .and_then(|r| r.job.in_flight.clone())
                    .map_or(now, |c| c.scheduled_for);
                return Err(JobError::RunInFlight {
                    job: name.clone(),
                    run_id: run_id.clone(),
                    scheduled_for,
                });
            }
            _ => {
                return Err(JobError::illegal_transition(
                    name.clone(),
                    current.label(),
                    "trigger",
                    "trigger requires an enabled, idle job",
                    Some(format!("clockwork job enable {name}")),
                ));
            }
        }

        Ok(PlannedChange {
            revision: snapshot.revision(),
            operation: "trigger",
            job: name.clone(),
            current_state: current.clone(),
            expected_state: Some(current),
            changes: vec![Change::TriggerRun],
            external_effect: ExternalEffect::ImmediateTrigger {
                action: source.definition.action.kind(),
            },
            definition: None,
            next_run: None,
            runtime: None,
        })
    }

    /// One validation path for every operation: schedule grammar, action
    /// policy, and generic profile resolution.
    fn validate_definition(
        definition: &JobDefinition,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<PlannedRuntimeDefinition, JobError> {
        definition.validate(now, snapshot.allow_insecure_http)?;
        profile_contract(
            definition,
            &snapshot.agents,
            snapshot.default_agent.as_deref(),
        )?;

        let parsed = parse_schedule(&definition.schedule, now)
            .map_err(|e| JobError::invalid_input(e.to_string()))?;
        let schedule = parsed.to_job_schedule();

        // Enablement and scheduling require a future next run. One-shot
        // fire times are absolute from parse time; a stale one-shot is an
        // error, not silent drift.
        if let JobSchedule::OneShot { fire_at } = schedule {
            if fire_at <= now {
                return Err(JobError::invalid_input(format!(
                    "one-shot schedule '{}' is in the past; provide a future time",
                    definition.schedule
                )));
            }
        }

        let action = definition
            .action
            .to_runtime_action(snapshot.allow_insecure_http)?;
        let raw = super::source::FsSourceStore::serialize(definition)?;
        let source_revision = snapshot
            .source
            .as_ref()
            .filter(|source| source.definition == *definition)
            .map_or_else(
                || super::state::content_revision(&raw),
                |source| source.revision.clone(),
            );

        Ok(PlannedRuntimeDefinition {
            schedule_input: definition.schedule.clone(),
            schedule,
            action,
            timeout_seconds: definition
                .timeout
                .unwrap_or(snapshot.default_timeout_seconds),
            tags: definition.tags.clone(),
            source_revision,
        })
    }
}
