use chrono::{DateTime, Utc};

use crate::engine::lock::FileLock;
use crate::model::invocation::Invocation;
use crate::util::id::new_run_id;

use super::error::JobError;
use super::inspect::{JobSnapshot, StateInspector};
use super::name::JobName;
use super::plan::{
    Change, ExternalEffect, JobOperation, JobPlanner, PlannedChange, PlannedRuntimeDefinition,
};
use super::profile::profile_contract;
use super::runtime::{FsRuntimeStore, RuntimeDefinition, RuntimeMutation, RuntimeStore};
use super::source::{FsSourceStore, SourceStore};
use super::state::{JobView, ManagedJobState, StateRevision, ValidationReport};

/// Verified outcome of a mutation.
#[derive(Debug, Clone)]
pub struct JobResult {
    pub operation: &'static str,
    pub job: JobName,
    pub changed: bool,
    /// Resulting state; `None` only for a successful delete.
    pub state: Option<ManagedJobState>,
    pub revision: StateRevision,
    pub external_effect: ExternalEffect,
}

/// Read job state, call the planner, and apply changes under the mutation lock.
/// Verify the stored result before reporting success.
pub struct JobService {
    inspector: StateInspector,
    sources: FsSourceStore,
    runtime: FsRuntimeStore,
}

impl Default for JobService {
    fn default() -> Self {
        Self::new()
    }
}

impl JobService {
    pub fn new() -> Self {
        Self {
            inspector: StateInspector::new(),
            sources: FsSourceStore,
            runtime: FsRuntimeStore,
        }
    }

    pub fn inspect(&self, name: &JobName, now: DateTime<Utc>) -> Result<JobView, JobError> {
        self.inspector.view(name, now)
    }

    /// Load the complete parsed definition before the CLI applies an update
    /// patch. The planner still reloads and validates it under the lock.
    pub fn definition(&self, name: &JobName) -> Result<super::definition::JobDefinition, JobError> {
        let snapshot = self.inspector.snapshot(name)?;
        snapshot
            .source
            .map(|source| source.definition)
            .ok_or_else(|| JobError::JobNotFound(super::error::NotFound(name.clone())))
    }

    pub fn list(&self, now: DateTime<Utc>) -> Result<Vec<JobView>, JobError> {
        self.inspector
            .list()?
            .into_iter()
            .map(|(name, snapshot)| Self::view_from_snapshot(&name, &snapshot, now))
            .collect()
    }

    fn view_from_snapshot(
        name: &JobName,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<JobView, JobError> {
        let state = snapshot.derive_state(now)?;
        let (schedule_input, action_kind, tags) = snapshot.definition().map_or(
            (
                String::new(),
                super::definition::ActionKind::Command,
                Vec::new(),
            ),
            |d| (d.schedule.clone(), d.action.kind(), d.tags.clone()),
        );
        Ok(JobView {
            name: name.clone(),
            activation: snapshot
                .activation()
                .unwrap_or(super::state::Activation::Disabled),
            revision: snapshot.revision(),
            schedule_input,
            action_kind,
            tags,
            state,
        })
    }

    /// Validate sources with the same parser used by mutation commands.
    /// Report malformed files individually so one error does not hide other results.
    pub fn validate(
        &self,
        name: Option<&JobName>,
        now: DateTime<Utc>,
    ) -> Result<ValidationReport, JobError> {
        let config =
            crate::store::config::load_config().map_err(|error| JobError::RuntimeFailure {
                message: format!("{error:#}"),
            })?;
        let names = match name {
            Some(name) => vec![name.clone()],
            None => FsSourceStore::names()?,
        };

        let mut jobs = Vec::new();
        for job_name in names {
            let result = match self.sources.load(&job_name) {
                Ok(Some(source)) => match self.runtime.snapshot(&job_name) {
                    Ok(runtime) => {
                        // A completed or due one-shot remains a valid source.
                        // Its runtime creation time proves that the stored
                        // absolute schedule was future when installed.
                        let validation_time = runtime
                            .as_ref()
                            .map_or(now, |runtime| runtime.job.created_at);
                        let definition_check = source
                            .definition
                            .validate(validation_time, config.allow_insecure_http)
                            .map_err(|error| error.to_string());
                        let profile_check = profile_contract(
                            &source.definition,
                            &config.agents,
                            config.default_agent.as_deref(),
                        )
                        .map_err(|error| error.to_string());
                        definition_check.and(profile_check)
                    }
                    Err(error) => Err(error.to_string()),
                },
                Ok(None) if name.is_some() => {
                    return Err(JobError::JobNotFound(super::error::NotFound(job_name)));
                }
                Ok(None) => continue,
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(()) => jobs.push(super::state::JobValidation {
                    job: job_name.as_str().to_string(),
                    valid: true,
                    errors: Vec::new(),
                }),
                Err(error) => jobs.push(super::state::JobValidation {
                    job: job_name.as_str().to_string(),
                    valid: false,
                    errors: vec![error],
                }),
            }
        }

        Ok(ValidationReport {
            ok: jobs.iter().all(|job| job.valid),
            jobs,
        })
    }

    /// Plan against the current inspected state.
    pub fn plan(
        &self,
        operation: &JobOperation,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let snapshot = self.inspector.snapshot(operation.name())?;
        JobPlanner::plan(operation, &snapshot, now)
    }

    /// Plan only if the current state still matches a reviewed revision.
    /// Execution repeats this check under the mutation lock.
    pub fn plan_at_revision(
        &self,
        operation: &JobOperation,
        expected_revision: &str,
        now: DateTime<Utc>,
    ) -> Result<PlannedChange, JobError> {
        let snapshot = self.inspector.snapshot(operation.name())?;
        let planned = JobPlanner::plan(operation, &snapshot, now)?;
        let actual = planned.revision.combined();
        if actual != expected_revision {
            return Err(JobError::RevisionConflict {
                job: Some(operation.name().clone()),
                expected: expected_revision.to_string(),
                actual,
            });
        }
        Ok(planned)
    }

    /// Execute a validated operation. When `expected_revision` is provided
    /// (from a prior dry run), the mutation applies only if the inspected
    /// revision still matches.
    pub fn execute(
        &self,
        operation: &JobOperation,
        expected_revision: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<JobResult, JobError> {
        let (snapshot, planned, trigger_run_id) = {
            let _lock = FileLock::state().map_err(|e| JobError::RuntimeFailure {
                message: format!("{e:#}"),
            })?;

            // Reload under the lock: preview state may be stale.
            let snapshot = self.inspector.snapshot(operation.name())?;

            let mut planned = JobPlanner::plan(operation, &snapshot, now)?;
            if let Some(expected) = expected_revision {
                let actual = planned.revision.combined();
                if actual != expected {
                    return Err(JobError::RevisionConflict {
                        job: Some(operation.name().clone()),
                        expected: expected.to_string(),
                        actual,
                    });
                }
            }
            if planned.is_noop() {
                return Ok(JobResult {
                    operation: planned.operation,
                    job: planned.job.clone(),
                    changed: false,
                    state: planned.expected_state,
                    revision: snapshot.revision(),
                    external_effect: planned.external_effect,
                });
            }

            // Do not create directories before the revision and policy guards
            // pass. Every pre-mutation rejection must leave storage untouched.
            Self::ensure_mutation_dirs()?;

            if !matches!(operation, JobOperation::Trigger(_)) {
                if let Err(error) = self.coordinate(&snapshot, &mut planned, now) {
                    return Err(self.with_mutation_progress(&snapshot, &planned, error));
                }

                // Postcondition: re-inspect and verify the derived state.
                let verified = self
                    .verify_postcondition(&planned, now)
                    .map_err(|error| self.with_mutation_progress(&snapshot, &planned, error))?;

                return Ok(JobResult {
                    operation: planned.operation,
                    job: planned.job.clone(),
                    changed: true,
                    state: verified,
                    revision: self.inspector.snapshot(&planned.job)?.revision(),
                    external_effect: planned.external_effect,
                });
            }

            // A public trigger becomes visible as in-flight before we release
            // the coordination lock. Update and delete therefore reject it
            // from the instant confirmation succeeds until the executor has
            // recorded the matching completion.
            let run_id = new_run_id();
            let expected = self.current_runtime_revision(&planned.job)?;
            self.runtime.apply(
                &planned.job,
                RuntimeMutation::ClaimRun {
                    run_id: run_id.clone(),
                    scheduled_for: now,
                },
                expected.as_deref(),
            )?;

            // The executor needs this lock to record completion and history.
            // Keep the revision check, lifecycle guard, and claim atomic;
            // then release it before the immediate external effect starts.
            (snapshot, planned, run_id)
        };

        if let Err(error) = Self::trigger_now(&planned.job, trigger_run_id, now) {
            return Err(self.with_mutation_progress(&snapshot, &planned, error));
        }

        let verified = self
            .verify_postcondition(&planned, now)
            .map_err(|error| self.with_mutation_progress(&snapshot, &planned, error))?;

        Ok(JobResult {
            operation: planned.operation,
            job: planned.job.clone(),
            changed: true,
            state: verified,
            revision: self.inspector.snapshot(&planned.job)?.revision(),
            external_effect: planned.external_effect,
        })
    }

    fn ensure_mutation_dirs() -> Result<(), JobError> {
        crate::store::paths::ensure_dirs().map_err(|error| JobError::RuntimeFailure {
            message: format!("{error:#}"),
        })?;
        let jobs_dir = super::source::jobs_dir()?;
        std::fs::create_dir_all(&jobs_dir).map_err(|error| JobError::SourceFailure {
            message: format!("failed to create {}: {error}", jobs_dir.display()),
        })?;
        crate::store::paths::set_dir_permissions(&jobs_dir).map_err(|error| {
            JobError::SourceFailure {
                message: format!("{error:#}"),
            }
        })?;
        Ok(())
    }

    /// Apply the planned steps in their safe order.
    // Keep the steps together so their execution order is visible.
    #[allow(clippy::too_many_lines)]
    fn coordinate(
        &self,
        snapshot: &JobSnapshot,
        planned: &mut PlannedChange,
        now: DateTime<Utc>,
    ) -> Result<(), JobError> {
        let mut runtime = planned.runtime.clone();
        let mut written_source_revision: Option<String> =
            snapshot.source.as_ref().map(|s| s.revision.clone());

        for change in &planned.changes {
            match change {
                Change::WriteSource => {
                    let definition =
                        planned
                            .definition
                            .clone()
                            .ok_or_else(|| JobError::RuntimeFailure {
                                message: "plan missing definition for source write".to_string(),
                            })?;
                    let revision = self
                        .sources
                        .write_atomic(&definition, written_source_revision.as_deref())?;
                    written_source_revision = Some(revision);
                }
                Change::CreateRuntimeDisabled => {
                    let defn = Self::runtime_definition(planned, runtime.take())?;
                    // Idempotent retry: if already installed, skip.
                    let current = self.runtime.snapshot(&planned.job)?;
                    if current.is_none() {
                        self.runtime.apply(
                            &planned.job,
                            RuntimeMutation::CreateDisabled(defn),
                            None,
                        )?;
                    }
                }
                Change::UpdateRuntimeDefinition => {
                    let defn = Self::runtime_definition(planned, runtime.take())?;
                    let expected = self.current_runtime_revision(&planned.job)?;
                    self.runtime.apply(
                        &planned.job,
                        RuntimeMutation::UpdateDefinition(defn),
                        expected.as_deref(),
                    )?;
                }
                Change::DisableScheduling => {
                    let expected = self.current_runtime_revision(&planned.job)?;
                    self.runtime.apply(
                        &planned.job,
                        RuntimeMutation::Disable,
                        expected.as_deref(),
                    )?;
                }
                Change::EnableScheduling => {
                    let expected = self.current_runtime_revision(&planned.job)?;
                    self.runtime.apply(
                        &planned.job,
                        RuntimeMutation::Enable { activated_at: now },
                        expected.as_deref(),
                    )?;
                }
                Change::ReplaceRuntimeGeneration => {
                    let defn = Self::runtime_definition(planned, runtime.take())?;
                    let expected = self.current_runtime_revision(&planned.job)?;
                    self.runtime.apply(
                        &planned.job,
                        RuntimeMutation::ReplaceGeneration(defn),
                        expected.as_deref(),
                    )?;
                }
                Change::RemoveRuntime => {
                    // Already gone: safe retry.
                    if self.runtime.snapshot(&planned.job)?.is_some() {
                        let expected = self.current_runtime_revision(&planned.job)?;
                        self.runtime.apply(
                            &planned.job,
                            RuntimeMutation::RemoveIdle,
                            expected.as_deref(),
                        )?;
                    }
                }
                Change::RemoveSource => {
                    let Some(expected) = written_source_revision
                        .clone()
                        .or_else(|| snapshot.source.as_ref().map(|s| s.revision.clone()))
                    else {
                        continue;
                    };
                    // Absent already: safe retry.
                    if self.sources.load(&planned.job)?.is_some() {
                        self.sources.remove_atomic(&planned.job, &expected)?;
                    }
                }
                Change::TriggerRun => {
                    return Err(JobError::RuntimeFailure {
                        message: "trigger execution must run after its durable claim".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Preserve the difference between a planner rejection and a failed
    /// multi-step mutation. Snapshot revisions cover the managed source and
    /// complete runtime object, so this reports `changed: true` exactly when
    /// an earlier step reached durable state.
    fn with_mutation_progress(
        &self,
        before: &JobSnapshot,
        planned: &PlannedChange,
        error: JobError,
    ) -> JobError {
        let changed = self
            .inspector
            .snapshot(&planned.job)
            .map(|after| after.revision() != before.revision())
            .unwrap_or(false);
        if changed {
            JobError::MutationFailed {
                job: planned.job.clone(),
                operation: planned.operation,
                message: error.to_string(),
            }
        } else {
            error
        }
    }

    fn runtime_definition(
        planned: &PlannedChange,
        payload: Option<PlannedRuntimeDefinition>,
    ) -> Result<RuntimeDefinition, JobError> {
        let payload = payload.ok_or_else(|| JobError::RuntimeFailure {
            message: "plan missing runtime payload".to_string(),
        })?;
        Ok(RuntimeDefinition {
            name: planned.job.clone(),
            schedule_input: payload.schedule_input,
            schedule: payload.schedule,
            action: payload.action,
            timeout_seconds: payload.timeout_seconds,
            tags: payload.tags,
            source_revision: payload.source_revision,
        })
    }

    fn current_runtime_revision(&self, name: &JobName) -> Result<Option<String>, JobError> {
        Ok(self
            .runtime
            .snapshot(name)?
            .map(|r| super::runtime::runtime_revision(&r.job)))
    }

    /// Run the already-claimed action through the executor. Trigger never
    /// falls back for missed schedules; it is the explicit immediate path.
    fn trigger_now(
        name: &JobName,
        run_id: String,
        scheduled_for: DateTime<Utc>,
    ) -> Result<(), JobError> {
        let invocation = Invocation::manual(name.as_str(), run_id, scheduled_for);
        let disposition =
            crate::engine::executor::execute_invocation(&invocation).map_err(|error| {
                JobError::RuntimeFailure {
                    message: format!("{error:#}"),
                }
            })?;
        if !disposition.process_succeeded() {
            return Err(JobError::RuntimeFailure {
                message: "triggered action failed before normal process completion".to_string(),
            });
        }
        Ok(())
    }

    /// Verify the postcondition after mutation: derive the public state from
    /// fresh storage and compare against the plan's expectation.
    fn verify_postcondition(
        &self,
        planned: &PlannedChange,
        now: DateTime<Utc>,
    ) -> Result<Option<ManagedJobState>, JobError> {
        let snapshot = self.inspector.snapshot(&planned.job)?;

        if planned.operation == "trigger" {
            return Self::verify_trigger_postcondition(planned, &snapshot, now);
        }

        if planned.expected_state.is_none() {
            // Delete: verify absence.
            if snapshot.source.is_some() || snapshot.runtime.is_some() {
                return Err(JobError::PostconditionFailed {
                    job: planned.job.clone(),
                    expected: "absent".to_string(),
                    actual: "still present".to_string(),
                });
            }
            return Ok(None);
        }

        let actual = snapshot.derive_state(now)?;
        let expected = planned.expected_state.clone().unwrap_or_default();

        if actual != expected {
            return Err(JobError::PostconditionFailed {
                job: planned.job.clone(),
                expected: format!("{expected:?}"),
                actual: format!("{actual:?}"),
            });
        }
        Ok(Some(actual))
    }

    /// A manual trigger updates the run record. Recurring jobs
    /// remain scheduled, while one-time jobs complete. Both outcomes must
    /// retain the previewed source revision and runtime generation.
    fn verify_trigger_postcondition(
        planned: &PlannedChange,
        snapshot: &JobSnapshot,
        now: DateTime<Utc>,
    ) -> Result<Option<ManagedJobState>, JobError> {
        let expected = planned
            .expected_state
            .as_ref()
            .ok_or_else(|| JobError::RuntimeFailure {
                message: "trigger plan has no expected scheduled state".to_string(),
            })?;
        let ManagedJobState::Scheduled {
            source_revision,
            runtime_generation,
            ..
        } = expected
        else {
            return Err(JobError::RuntimeFailure {
                message: "trigger plan expected a non-scheduled state".to_string(),
            });
        };

        let actual = snapshot.derive_state(now)?;
        let matches_generation = match &actual {
            ManagedJobState::Scheduled {
                source_revision: actual_revision,
                runtime_generation: actual_generation,
                next_run,
            } => {
                actual_revision == source_revision
                    && actual_generation == runtime_generation
                    && *next_run > now
            }
            ManagedJobState::Completed {
                source_revision: actual_revision,
                runtime_generation: actual_generation,
                ..
            } => actual_revision == source_revision && actual_generation == runtime_generation,
            ManagedJobState::Draft { .. }
            | ManagedJobState::Disabled { .. }
            | ManagedJobState::Running { .. } => false,
        };

        if !matches_generation {
            return Err(JobError::PostconditionFailed {
                job: planned.job.clone(),
                expected: format!(
                    "trigger completion for source revision {source_revision} and generation {runtime_generation}"
                ),
                actual: format!("{actual:?}"),
            });
        }
        Ok(Some(actual))
    }
}

impl Default for ManagedJobState {
    fn default() -> Self {
        ManagedJobState::Draft {
            source_revision: String::new(),
        }
    }
}
