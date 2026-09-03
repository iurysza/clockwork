use chrono::{DateTime, Utc};

use crate::model::action::Action;
use crate::model::job::{Job, JobStatus};
use crate::model::run_record::{LastRun, RunStatus};
use crate::model::schedule::JobSchedule;
use crate::store::state;

use super::error::JobError;
use super::name::JobName;
use super::state::content_revision;

/// Definition payload the runtime job is built from. Carries everything the
/// runtime needs and nothing from the public CLI surface.
#[derive(Debug, Clone)]
pub struct RuntimeDefinition {
    pub name: JobName,
    pub schedule_input: String,
    pub schedule: JobSchedule,
    pub action: Action,
    pub timeout_seconds: u64,
    pub tags: Vec<String>,
    pub source_revision: String,
}

/// A runtime job plus its content revision.
#[derive(Debug, Clone)]
pub struct VersionedRuntimeJob {
    pub job: Job,
    _revision: String,
}

pub fn runtime_revision(job: &Job) -> String {
    let bytes = serde_json::to_vec(job).expect("runtime jobs are JSON serializable");
    content_revision(&bytes)
}

/// Crate-private runtime mutations. The public parser cannot construct this —
/// only the application service and the scheduler drive it.
pub(crate) enum RuntimeMutation {
    CreateDisabled(RuntimeDefinition),
    UpdateDefinition(RuntimeDefinition),
    Enable {
        activated_at: DateTime<Utc>,
    },
    Disable,
    ReplaceGeneration(RuntimeDefinition),
    RemoveIdle,
    ClaimRun {
        run_id: String,
        scheduled_for: DateTime<Utc>,
    },
    CompleteRun {
        run_id: String,
        scheduled_for: DateTime<Utc>,
        advance_schedule: bool,
        last_run: LastRun,
    },
}

/// Private runtime storage over the durable job state file. Callers must
/// hold the state lock across `apply`.
pub(crate) trait RuntimeStore {
    fn snapshot(&self, name: &JobName) -> Result<Option<VersionedRuntimeJob>, JobError>;
    // One lock-scoped transaction: existence handling, the optimistic
    // revision check, and every mutation arm in sequence. Splitting it
    // would scatter the atomicity story.
    #[allow(clippy::too_many_lines)]
    fn apply(
        &self,
        name: &JobName,
        mutation: RuntimeMutation,
        expected: Option<&str>,
    ) -> Result<VersionedRuntimeJob, JobError>;
}

pub struct FsRuntimeStore;

impl FsRuntimeStore {
    fn find_job<'a>(
        jobs: &'a std::collections::BTreeMap<String, Job>,
        name: &JobName,
    ) -> Option<(&'a String, &'a Job)> {
        jobs.get_key_value(name.as_str()).or_else(|| {
            jobs.iter()
                .find(|(_, job)| job.name.as_deref() == Some(name.as_str()))
        })
    }
}

impl RuntimeStore for FsRuntimeStore {
    fn snapshot(&self, name: &JobName) -> Result<Option<VersionedRuntimeJob>, JobError> {
        let state = state::load_state().map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;
        Ok(
            Self::find_job(&state.jobs, name).map(|(_, job)| VersionedRuntimeJob {
                job: job.clone(),
                _revision: runtime_revision(job),
            }),
        )
    }

    // One lock-scoped transaction: existence handling, the optimistic
    // revision check, and every mutation arm in sequence. Splitting it
    // would scatter the atomicity story.
    #[allow(clippy::too_many_lines)]
    fn apply(
        &self,
        name: &JobName,
        mutation: RuntimeMutation,
        expected: Option<&str>,
    ) -> Result<VersionedRuntimeJob, JobError> {
        let mut outcome: Option<VersionedRuntimeJob> = None;
        state::update_state(|s| {
            let Some((job_id, mut job)) = Self::find_job(&s.jobs, name)
                .map(|(id, job)| (id.clone(), job.clone()))
            else {
                // Create is the only mutation allowed on an absent job.
                return match mutation {
                    RuntimeMutation::CreateDisabled(defn) => {
                        let job = build_runtime_job(&defn);
                        let versioned = VersionedRuntimeJob {
                            _revision: runtime_revision(&job),
                            job: job.clone(),
                        };
                        s.jobs.insert(job_id_of(&defn.name), job);
                        outcome = Some(versioned);
                        Ok(())
                    }
                    _ => Err(anyhow::anyhow!("runtime job '{}' not found", name.as_str())),
                };
            };

            let current_revision = runtime_revision(&job);
            if let Some(exp) = expected {
                if exp != current_revision {
                    return Err(anyhow::anyhow!(
                        "runtime revision conflict for '{}': expected {exp}, found {current_revision}",
                        name.as_str()
                    ));
                }
            }

            let mut changed = true;
            match mutation {
                RuntimeMutation::CreateDisabled(defn) => {
                    // Idempotent retry: same source revision already installed.
                    if job.source_revision.as_deref() == Some(defn.source_revision.as_str()) {
                        changed = false;
                    } else {
                        return Err(anyhow::anyhow!(
                            "runtime job '{}' already exists with a different source revision",
                            name.as_str()
                        ));
                    }
                }
                RuntimeMutation::UpdateDefinition(defn) => {
                    if job.in_flight.is_some() {
                        return Err(
                            anyhow::anyhow!("job '{}' has a run in flight", name.as_str()),
                        );
                    }
                    apply_definition(&mut job, &defn);
                }
                RuntimeMutation::Enable { activated_at } => {
                    if job.in_flight.is_some() {
                        return Err(
                            anyhow::anyhow!("job '{}' has a run in flight", name.as_str()),
                        );
                    }
                    job.status = JobStatus::Active;
                    if !job.is_one_shot() {
                        // Activation starts a new recurring window. Time spent
                        // disabled must not become an immediate missed run.
                        job.last_scheduled_at = Some(activated_at);
                    }
                }
                RuntimeMutation::Disable => {
                    job.status = JobStatus::Paused;
                }
                RuntimeMutation::ReplaceGeneration(defn) => {
                    if job.in_flight.is_some() {
                        return Err(
                            anyhow::anyhow!("job '{}' has a run in flight", name.as_str()),
                        );
                    }
                    job.generation = job.generation.saturating_add(1);
                    job.run_count = 0;
                    job.last_run = None;
                    job.last_scheduled_at = None;
                    job.completed_at = None;
                    job.consecutive_failures = 0;
                    job.in_flight = None;
                    apply_definition(&mut job, &defn);
                    job.status = JobStatus::Paused;
                }
                RuntimeMutation::RemoveIdle => {
                    if job.in_flight.is_some() {
                        return Err(
                            anyhow::anyhow!("job '{}' has a run in flight", name.as_str()),
                        );
                    }
                    s.jobs.remove(&job_id);
                    outcome = Some(VersionedRuntimeJob {
                        _revision: current_revision,
                        job,
                    });
                    return Ok(());
                }
                RuntimeMutation::ClaimRun { .. } | RuntimeMutation::CompleteRun { .. } => {
                    apply_runtime_mutation(&mut job, mutation)?;
                }
            }

            job.updated_at = Utc::now();
            if changed {
                outcome = Some(VersionedRuntimeJob {
                    _revision: runtime_revision(&job),
                    job: job.clone(),
                });
            } else {
                outcome = Some(VersionedRuntimeJob {
                    _revision: current_revision,
                    job: job.clone(),
                });
            }
            s.jobs.insert(job_id, job);
            Ok(())
        })
        .map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;

        outcome.ok_or_else(|| JobError::RuntimeFailure {
            message: "mutation produced no result".to_string(),
        })
    }
}

fn job_id_of(name: &JobName) -> String {
    name.as_str().to_string()
}

impl FsRuntimeStore {
    /// Narrow scheduler API: durably claim a run before spawning the
    /// executor. Returns `Ok(false)` when another claim is already held;
    /// the dispatcher treats that as a lost race and does not launch.
    /// Callers must hold the global state lock.
    pub(crate) fn claim_run(
        job_id: &str,
        run_id: String,
        scheduled_for: DateTime<Utc>,
    ) -> Result<bool, JobError> {
        let mut claimed = false;
        state::update_state(|state| {
            let Some(job) = state.jobs.get_mut(job_id) else {
                return Ok(());
            };
            if job.in_flight.is_none() {
                apply_runtime_mutation(
                    job,
                    RuntimeMutation::ClaimRun {
                        run_id,
                        scheduled_for,
                    },
                )?;
                claimed = true;
            }
            Ok(())
        })
        .map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;
        Ok(claimed)
    }

    /// Narrow scheduler API: record the completion of the run `run_id`.
    /// Absent jobs (removed by other means) are a no-op so a finishing
    /// executor never recreates state. Callers must hold the state lock.
    pub(crate) fn complete_run(
        job_id: &str,
        run_id: &str,
        scheduled_for: DateTime<Utc>,
        advance_schedule: bool,
        last_run: LastRun,
    ) -> Result<(), JobError> {
        state::update_state(|state| {
            let Some(job) = state.jobs.get_mut(job_id) else {
                return Ok(());
            };
            apply_runtime_mutation(
                job,
                RuntimeMutation::CompleteRun {
                    run_id: run_id.to_string(),
                    scheduled_for,
                    advance_schedule,
                    last_run,
                },
            )?;
            Ok(())
        })
        .map_err(|e| JobError::RuntimeFailure {
            message: format!("{e:#}"),
        })?;
        Ok(())
    }
}

/// Apply one runtime mutation to an existing job. Shared by the store's
/// `apply` and the scheduler's narrow claim/complete APIs so there is one
/// implementation for claims, completions, and run bookkeeping.
fn apply_runtime_mutation(job: &mut Job, mutation: RuntimeMutation) -> Result<(), JobError> {
    match mutation {
        RuntimeMutation::ClaimRun {
            run_id,
            scheduled_for,
        } => {
            if job.status != JobStatus::Active {
                let name = JobName::parse(job.name.as_deref().unwrap_or(&job.id))
                    .map_err(JobError::invalid_input)?;
                return Err(JobError::illegal_transition(
                    name,
                    "disabled",
                    "claim",
                    "only an enabled job can claim a run",
                    None,
                ));
            }
            if job.in_flight.is_some() {
                return Err(JobError::RunInFlight {
                    job: JobName::parse(&job.id).unwrap_or_else(|_| JobName::parse("job").unwrap()),
                    run_id,
                    scheduled_for,
                });
            }
            job.in_flight = Some(crate::model::job::ScheduledClaim {
                run_id,
                scheduled_for,
                claimed_at: Utc::now(),
            });
            job.updated_at = Utc::now();
        }
        RuntimeMutation::CompleteRun {
            run_id,
            scheduled_for,
            advance_schedule,
            last_run,
        } => {
            if last_run.run_id != run_id {
                return Err(JobError::integrity(
                    JobName::parse(job.name.as_deref().unwrap_or(&job.id)).ok(),
                    "completion record does not match the claimed run",
                ));
            }
            // A stale executor must not update counters, history summaries,
            // or one-shot state after another run owns the claim.
            match &job.in_flight {
                Some(claim) if claim.run_id == run_id && claim.scheduled_for == scheduled_for => {
                    job.in_flight = None;
                }
                Some(claim) => {
                    let job_name = JobName::parse(job.name.as_deref().unwrap_or(&job.id))
                        .map_err(JobError::invalid_input)?;
                    return Err(JobError::RunInFlight {
                        job: job_name,
                        run_id: claim.run_id.clone(),
                        scheduled_for: claim.scheduled_for,
                    });
                }
                None if job.managed_by.as_deref() == Some("managed-job") => {
                    return Err(JobError::integrity(
                        JobName::parse(job.name.as_deref().unwrap_or(&job.id)).ok(),
                        format!("run '{run_id}' no longer owns an in-flight claim"),
                    ));
                }
                None => {}
            }
            job.run_count = job.run_count.saturating_add(1);
            if advance_schedule {
                job.last_scheduled_at = Some(
                    job.last_scheduled_at
                        .map_or(scheduled_for, |current| current.max(scheduled_for)),
                );
            }
            job.updated_at = last_run.finished_at;
            let status = last_run.status;
            job.last_run = Some(last_run);
            if status.should_trigger_fallback() {
                job.consecutive_failures = job.consecutive_failures.saturating_add(1);
            } else if status == RunStatus::Success {
                job.consecutive_failures = 0;
            }
            if job.is_one_shot() && !status.is_internal_error() {
                job.status = JobStatus::Completed;
                job.completed_at = Some(job.updated_at);
            }
        }
        _ => {
            return Err(JobError::RuntimeFailure {
                message: "mutation is only valid through the store apply path".to_string(),
            });
        }
    }
    Ok(())
}

fn build_runtime_job(defn: &RuntimeDefinition) -> Job {
    let now = Utc::now();
    Job {
        id: job_id_of(&defn.name),
        name: Some(defn.name.as_str().to_string()),
        // Disabled by construction: the only public start path is enable.
        status: JobStatus::Paused,
        schedule_input: defn.schedule_input.clone(),
        schedule: defn.schedule.clone(),
        action: defn.action.clone(),
        timeout_seconds: defn.timeout_seconds,
        tags: defn.tags.clone(),
        created_at: now,
        updated_at: now,
        last_scheduled_at: None,
        last_run: None,
        run_count: 0,
        skip_remaining: 0,
        in_flight: None,
        on_failure: None,
        on_failure_shell: false,
        completed_at: None,
        consecutive_failures: 0,
        managed_by: Some("managed-job".to_string()),
        source_revision: Some(defn.source_revision.clone()),
        generation: 0,
    }
}

fn apply_definition(job: &mut Job, defn: &RuntimeDefinition) {
    let schedule_changed = job.schedule_input != defn.schedule_input;
    job.name = Some(defn.name.as_str().to_string());
    job.schedule_input.clone_from(&defn.schedule_input);
    job.schedule = defn.schedule.clone();
    job.action = defn.action.clone();
    job.timeout_seconds = defn.timeout_seconds;
    job.tags.clone_from(&defn.tags);
    job.source_revision = Some(defn.source_revision.clone());
    if schedule_changed {
        // Avoid a catch-up burst for the new schedule.
        job.last_scheduled_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::job::ScheduledClaim;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 11, 22, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
    }

    fn test_job(schedule: JobSchedule) -> Job {
        Job {
            id: "job".to_string(),
            name: Some("job".to_string()),
            status: JobStatus::Active,
            schedule_input: "schedule".to_string(),
            schedule,
            action: Action::Run {
                command: "true".to_string(),
                shell: false,
                workdir: None,
            },
            timeout_seconds: 30,
            tags: Vec::new(),
            created_at: at(0),
            updated_at: at(0),
            last_scheduled_at: None,
            last_run: None,
            run_count: 0,
            skip_remaining: 0,
            in_flight: None,
            on_failure: None,
            on_failure_shell: false,
            completed_at: None,
            consecutive_failures: 0,
            managed_by: Some("managed-job".to_string()),
            source_revision: Some("rev_src".to_string()),
            generation: 0,
        }
    }

    fn last_run(status: RunStatus) -> LastRun {
        LastRun {
            run_id: "run".to_string(),
            started_at: at(11),
            finished_at: at(12),
            status,
            exit_code: None,
            log_path: "logs/job/run.log".to_string(),
            error_message: None,
        }
    }

    #[test]
    fn claim_run_marks_in_flight_and_rejects_a_second_claim() {
        let mut job = test_job(JobSchedule::RecurringInterval { every_seconds: 10 });
        apply_runtime_mutation(
            &mut job,
            RuntimeMutation::ClaimRun {
                run_id: "run".to_string(),
                scheduled_for: at(10),
            },
        )
        .expect("first claim");
        assert_eq!(job.in_flight.as_ref().unwrap().run_id, "run");

        let second = apply_runtime_mutation(
            &mut job,
            RuntimeMutation::ClaimRun {
                run_id: "other".to_string(),
                scheduled_for: at(20),
            },
        );
        assert!(matches!(second, Err(JobError::RunInFlight { .. })));
        assert_eq!(job.in_flight.as_ref().unwrap().run_id, "run");
    }

    #[test]
    fn stale_completion_cannot_cross_another_runs_claim() {
        let mut job = test_job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.in_flight = Some(ScheduledClaim {
            run_id: "other".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        let result = apply_runtime_mutation(
            &mut job,
            RuntimeMutation::CompleteRun {
                run_id: "run".to_string(),
                scheduled_for: at(10),
                advance_schedule: true,
                last_run: last_run(RunStatus::Success),
            },
        );

        assert!(matches!(result, Err(JobError::RunInFlight { .. })));
        assert_eq!(job.in_flight.as_ref().unwrap().run_id, "other");
        assert_eq!(job.run_count, 0);
        assert_eq!(job.last_scheduled_at, None);
        assert!(job.last_run.is_none());
    }

    #[test]
    fn manual_completion_preserves_the_recurring_schedule_anchor() {
        let mut job = test_job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.last_scheduled_at = Some(at(5));
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        apply_runtime_mutation(
            &mut job,
            RuntimeMutation::CompleteRun {
                run_id: "run".to_string(),
                scheduled_for: at(10),
                advance_schedule: false,
                last_run: last_run(RunStatus::Success),
            },
        )
        .expect("manual completion");

        assert_eq!(job.last_scheduled_at, Some(at(5)));
        assert_eq!(job.run_count, 1);
    }

    #[test]
    fn complete_run_finishes_a_one_shot_job_but_not_on_internal_error() {
        let mut job = test_job(JobSchedule::OneShot { fire_at: at(10) });
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        apply_runtime_mutation(
            &mut job,
            RuntimeMutation::CompleteRun {
                run_id: "run".to_string(),
                scheduled_for: at(10),
                advance_schedule: true,
                last_run: last_run(RunStatus::Success),
            },
        )
        .expect("completion");
        assert_eq!(job.status, JobStatus::Completed);
        assert!(job.in_flight.is_none());
        assert!(job.completed_at.is_some());

        let mut broken = test_job(JobSchedule::OneShot { fire_at: at(10) });
        broken.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        apply_runtime_mutation(
            &mut broken,
            RuntimeMutation::CompleteRun {
                run_id: "run".to_string(),
                scheduled_for: at(10),
                advance_schedule: true,
                last_run: last_run(RunStatus::InternalError),
            },
        )
        .expect("internal error completion");
        assert_eq!(broken.status, JobStatus::Active);
    }

    #[test]
    fn complete_run_tracks_consecutive_failures() {
        let mut job = test_job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        apply_runtime_mutation(
            &mut job,
            RuntimeMutation::CompleteRun {
                run_id: "run".to_string(),
                scheduled_for: at(10),
                advance_schedule: true,
                last_run: last_run(RunStatus::Failed),
            },
        )
        .expect("failed completion");
        assert_eq!(job.consecutive_failures, 1);
        job.in_flight = Some(ScheduledClaim {
            run_id: "run2".to_string(),
            scheduled_for: at(20),
            claimed_at: at(20),
        });
        let mut successful_run = last_run(RunStatus::Success);
        successful_run.run_id = "run2".to_string();
        apply_runtime_mutation(
            &mut job,
            RuntimeMutation::CompleteRun {
                run_id: "run2".to_string(),
                scheduled_for: at(20),
                advance_schedule: true,
                last_run: successful_run,
            },
        )
        .expect("successful completion");
        assert_eq!(job.consecutive_failures, 0);
    }
}
