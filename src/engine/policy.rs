use chrono::{DateTime, Duration, Utc};

use crate::model::invocation::{Invocation, InvocationSource, RunAttempt};
use crate::model::job::{Job, JobStatus, ScheduledClaim};
use crate::model::run_record::{LastRun, RunRecord, RunStatus};
use crate::schedule::occurrence::{OccurrenceError, due_after, latest_due};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionAvailability {
    Available,
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    InactiveJob,
    StaleOrDuplicateClaim,
    ScheduledLockBusy,
}

#[derive(Debug, Clone)]
pub enum RunDecision {
    Start(RunAttempt),
    Skip(RunRecord),
    Ignore(IgnoreReason),
}

pub fn decide_run(
    job: &Job,
    invocation: &Invocation,
    availability: ExecutionAvailability,
    observed_at: DateTime<Utc>,
) -> RunDecision {
    if job.status != JobStatus::Active {
        return RunDecision::Ignore(IgnoreReason::InactiveJob);
    }

    if let InvocationSource::Scheduled { occurrence_at } = invocation.source {
        let claim_matches = job.in_flight.as_ref().is_some_and(|claim| {
            claim.run_id == invocation.run_id && claim.scheduled_for == occurrence_at
        });
        if !claim_matches {
            return RunDecision::Ignore(IgnoreReason::StaleOrDuplicateClaim);
        }
    }

    if availability == ExecutionAvailability::Busy {
        return match invocation.source {
            InvocationSource::Manual => RunDecision::Skip(RunRecord {
                run_id: invocation.run_id.clone(),
                job_id: invocation.job_id.clone(),
                trigger: invocation.source.trigger(),
                scheduled_for: invocation.recorded_for(),
                started_at: observed_at,
                finished_at: observed_at,
                status: RunStatus::SkippedOverlap,
                exit_code: None,
                log_path: String::new(),
                failed_run_id: None,
                error_message: None,
            }),
            InvocationSource::Scheduled { .. } => {
                RunDecision::Ignore(IgnoreReason::ScheduledLockBusy)
            }
        };
    }

    RunDecision::Start(RunAttempt::from(invocation))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedExecution {
    NotRunning,
    Running,
}

#[derive(Debug, Clone)]
pub enum DispatchEffect {
    Launch(Invocation),
    RecordSkippedOverlap { scheduled_for: DateTime<Utc> },
}

#[derive(Debug, Clone)]
pub struct DispatchPlan {
    pub job: Job,
    pub changed: bool,
    pub effects: Vec<DispatchEffect>,
}

pub fn plan_dispatch(
    job: &Job,
    now: DateTime<Utc>,
    claimed_execution: ClaimedExecution,
    proposed_run_id: String,
) -> Result<DispatchPlan, OccurrenceError> {
    let mut planned = job.clone();
    let mut changed = false;
    let mut effects = Vec::new();

    if job.status != JobStatus::Active {
        return Ok(DispatchPlan {
            job: planned,
            changed,
            effects,
        });
    }

    if let Some(claim) = &job.in_flight {
        if claimed_execution == ClaimedExecution::NotRunning {
            return Ok(DispatchPlan {
                job: planned,
                changed,
                effects,
            });
        }

        let anchor = job
            .last_scheduled_at
            .unwrap_or(job.created_at)
            .max(claim.scheduled_for);
        for scheduled_for in due_after(&job.schedule, anchor, now)? {
            if planned.skip_remaining > 0 {
                planned.skip_remaining -= 1;
            } else {
                effects.push(DispatchEffect::RecordSkippedOverlap { scheduled_for });
            }
            planned.last_scheduled_at = Some(scheduled_for);
            planned.updated_at = now;
            changed = true;
        }

        return Ok(DispatchPlan {
            job: planned,
            changed,
            effects,
        });
    }

    if job.is_one_shot() && job.last_scheduled_at.is_some() {
        return Ok(DispatchPlan {
            job: planned,
            changed,
            effects,
        });
    }

    let anchor = job.last_scheduled_at.unwrap_or(job.created_at);
    let Some(scheduled_for) = latest_due(&job.schedule, anchor, now)? else {
        return Ok(DispatchPlan {
            job: planned,
            changed,
            effects,
        });
    };

    if planned.skip_remaining > 0 {
        planned.skip_remaining -= 1;
        planned.last_scheduled_at = Some(scheduled_for);
        planned.updated_at = now;
        changed = true;
    } else {
        planned.in_flight = Some(ScheduledClaim {
            run_id: proposed_run_id.clone(),
            scheduled_for,
            claimed_at: now,
        });
        planned.updated_at = now;
        changed = true;
        effects.push(DispatchEffect::Launch(Invocation::scheduled(
            job.id.clone(),
            proposed_run_id,
            scheduled_for,
        )));
    }

    Ok(DispatchPlan {
        job: planned,
        changed,
        effects,
    })
}

#[derive(Debug, Clone)]
pub enum ClaimRecovery {
    Keep,
    Recover { job: Box<Job>, record: RunRecord },
}

pub fn recover_claim(
    job: &Job,
    now: DateTime<Utc>,
    claimed_execution: ClaimedExecution,
    grace: Duration,
) -> ClaimRecovery {
    let Some(claim) = &job.in_flight else {
        return ClaimRecovery::Keep;
    };

    if now - claim.claimed_at < grace || claimed_execution == ClaimedExecution::Running {
        return ClaimRecovery::Keep;
    }

    let record = RunRecord {
        run_id: claim.run_id.clone(),
        job_id: job.id.clone(),
        trigger: crate::model::run_record::Trigger::Scheduled,
        scheduled_for: claim.scheduled_for,
        started_at: claim.claimed_at,
        finished_at: now,
        status: RunStatus::InternalError,
        exit_code: None,
        log_path: String::new(),
        failed_run_id: None,
        error_message: None,
    };
    let mut recovered = job.clone();
    recovered.in_flight = None;
    recovered.updated_at = now;
    recovered.last_run = Some(last_run_from_record(&record));

    ClaimRecovery::Recover {
        job: Box::new(recovered),
        record,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionExit {
    Exited { code: Option<i32> },
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Success { exit_code: i32 },
    Failed { exit_code: Option<i32> },
    Timeout,
    InternalError { safe_message: String },
}

impl RunOutcome {
    pub fn status(&self) -> RunStatus {
        match self {
            Self::Success { .. } => RunStatus::Success,
            Self::Failed { .. } => RunStatus::Failed,
            Self::Timeout => RunStatus::Timeout,
            Self::InternalError { .. } => RunStatus::InternalError,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Success { exit_code } => Some(*exit_code),
            Self::Failed { exit_code } => *exit_code,
            Self::Timeout | Self::InternalError { .. } => None,
        }
    }

    pub fn error_message(&self) -> Option<String> {
        match self {
            Self::InternalError { safe_message } => Some(safe_message.clone()),
            Self::Success { .. } | Self::Failed { .. } | Self::Timeout => None,
        }
    }
}

pub fn classify_outcome(result: Result<ActionExit, String>) -> RunOutcome {
    match result {
        Ok(ActionExit::Exited { code: Some(0) }) => RunOutcome::Success { exit_code: 0 },
        Ok(ActionExit::Exited { code }) => RunOutcome::Failed { exit_code: code },
        Ok(ActionExit::TimedOut) => RunOutcome::Timeout,
        Err(safe_message) => RunOutcome::InternalError { safe_message },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunTimes {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FailureRequest {
    pub job_id: String,
    pub failed_run_id: String,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub log_path: String,
    pub recorded_for: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CompletionPlan {
    pub record: RunRecord,
    pub failure: Option<FailureRequest>,
}

/// Build the history record and fallback request for a finished attempt.
/// Runtime bookkeeping (claim clearing, counters, one-shot completion) is
/// owned by the runtime store's `CompleteRun` mutation — this policy only
/// classifies the outcome.
pub fn complete_run(
    attempt: &RunAttempt,
    outcome: &RunOutcome,
    times: RunTimes,
    log_path: String,
) -> CompletionPlan {
    let status = outcome.status();
    let exit_code = outcome.exit_code();
    let error_message = outcome.error_message();
    let recorded_for = attempt.recorded_for();

    let record = RunRecord {
        run_id: attempt.run_id.clone(),
        job_id: attempt.job_id.clone(),
        trigger: attempt.trigger(),
        scheduled_for: recorded_for,
        started_at: times.started_at,
        finished_at: times.finished_at,
        status,
        exit_code,
        log_path: log_path.clone(),
        failed_run_id: None,
        error_message,
    };
    let failure = status.should_trigger_fallback().then(|| FailureRequest {
        job_id: attempt.job_id.clone(),
        failed_run_id: attempt.run_id.clone(),
        status,
        exit_code,
        log_path,
        recorded_for,
    });
    CompletionPlan { record, failure }
}

#[derive(Debug, Clone)]
pub enum ExecutionDisposition {
    Completed(RunOutcome),
    Skipped(RunRecord),
    Ignored(IgnoreReason),
}

impl ExecutionDisposition {
    pub fn process_succeeded(&self) -> bool {
        match self {
            Self::Completed(RunOutcome::InternalError { .. }) => false,
            Self::Completed(_) => true,
            Self::Skipped(record) => record.status == RunStatus::SkippedOverlap,
            Self::Ignored(reason) => matches!(
                reason,
                IgnoreReason::InactiveJob
                    | IgnoreReason::StaleOrDuplicateClaim
                    | IgnoreReason::ScheduledLockBusy
            ),
        }
    }
}

fn last_run_from_record(record: &RunRecord) -> LastRun {
    LastRun {
        run_id: record.run_id.clone(),
        started_at: record.started_at,
        finished_at: record.finished_at,
        status: record.status,
        exit_code: record.exit_code,
        log_path: record.log_path.clone(),
        error_message: record.error_message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::model::action::Action;
    use crate::model::schedule::JobSchedule;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 11, 22, 0, 0).unwrap() + Duration::seconds(seconds)
    }

    fn job(schedule: JobSchedule) -> Job {
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
            managed_by: None,
            source_revision: None,
            generation: 0,
        }
    }

    #[test]
    fn matching_scheduled_claim_starts() {
        let mut job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        let invocation = Invocation::scheduled("job", "run", at(10));

        assert!(matches!(
            decide_run(&job, &invocation, ExecutionAvailability::Available, at(11)),
            RunDecision::Start(_)
        ));
    }

    #[test]
    fn inactive_job_is_ignored() {
        let mut job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.status = JobStatus::Paused;
        let invocation = Invocation::manual("job", "manual", at(10));

        assert!(matches!(
            decide_run(&job, &invocation, ExecutionAvailability::Available, at(11)),
            RunDecision::Ignore(IgnoreReason::InactiveJob)
        ));
    }

    #[test]
    fn stale_scheduled_claim_is_ignored() {
        let job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        let invocation = Invocation::scheduled("job", "stale", at(10));

        assert!(matches!(
            decide_run(&job, &invocation, ExecutionAvailability::Available, at(11)),
            RunDecision::Ignore(IgnoreReason::StaleOrDuplicateClaim)
        ));
    }

    #[test]
    fn busy_scheduled_run_keeps_its_claim() {
        let mut job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        let invocation = Invocation::scheduled("job", "run", at(10));

        assert!(matches!(
            decide_run(&job, &invocation, ExecutionAvailability::Busy, at(11)),
            RunDecision::Ignore(IgnoreReason::ScheduledLockBusy)
        ));
    }

    #[test]
    fn busy_manual_run_is_recorded_as_overlap() {
        let job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        let invocation = Invocation::manual("job", "manual", at(11));

        let RunDecision::Skip(record) =
            decide_run(&job, &invocation, ExecutionAvailability::Busy, at(12))
        else {
            panic!("expected overlap record");
        };
        assert_eq!(record.status, RunStatus::SkippedOverlap);
        assert_eq!(record.trigger, crate::model::run_record::Trigger::Manual);
    }

    #[test]
    fn due_job_is_claimed_for_launch() {
        let job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        let plan = plan_dispatch(
            &job,
            at(15),
            ClaimedExecution::NotRunning,
            "run".to_string(),
        )
        .unwrap();

        assert!(plan.changed);
        assert_eq!(plan.job.in_flight.as_ref().unwrap().run_id, "run");
        assert!(matches!(
            plan.effects.as_slice(),
            [DispatchEffect::Launch(_)]
        ));
    }

    #[test]
    fn running_claim_records_each_missed_overlap() {
        let mut job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });
        let plan = plan_dispatch(
            &job,
            at(35),
            ClaimedExecution::Running,
            "unused".to_string(),
        )
        .unwrap();

        assert_eq!(plan.effects.len(), 2);
        assert_eq!(plan.job.last_scheduled_at, Some(at(30)));
    }

    #[test]
    fn stale_claim_recovery_does_not_increment_run_count() {
        let mut job = job(JobSchedule::RecurringInterval { every_seconds: 10 });
        job.in_flight = Some(ScheduledClaim {
            run_id: "run".to_string(),
            scheduled_for: at(10),
            claimed_at: at(10),
        });

        let ClaimRecovery::Recover { job, record } = recover_claim(
            &job,
            at(21),
            ClaimedExecution::NotRunning,
            Duration::seconds(10),
        ) else {
            panic!("expected claim recovery");
        };
        assert_eq!(record.status, RunStatus::InternalError);
        assert_eq!(job.run_count, 0);
        assert!(job.in_flight.is_none());
    }

    #[test]
    fn action_timeout_is_classified_without_an_exit_code() {
        assert_eq!(
            classify_outcome(Ok(ActionExit::TimedOut)),
            RunOutcome::Timeout
        );
    }

    #[test]
    fn successful_one_shot_completion_builds_a_success_record() {
        let attempt = RunAttempt::from(&Invocation::scheduled("job", "run", at(10)));
        let plan = complete_run(
            &attempt,
            &RunOutcome::Success { exit_code: 0 },
            RunTimes {
                started_at: at(11),
                finished_at: at(12),
            },
            "logs/job/run.log".to_string(),
        );

        assert_eq!(plan.record.status, RunStatus::Success);
        assert_eq!(plan.record.run_id, "run");
        assert!(plan.failure.is_none());
    }

    #[test]
    fn internal_error_builds_an_internal_error_record_and_requests_fallback() {
        let attempt = RunAttempt::from(&Invocation::scheduled("job", "run", at(10)));
        let plan = complete_run(
            &attempt,
            &RunOutcome::InternalError {
                safe_message: "process could not start".to_string(),
            },
            RunTimes {
                started_at: at(11),
                finished_at: at(12),
            },
            "logs/job/run.log".to_string(),
        );

        assert_eq!(plan.record.status, RunStatus::InternalError);
        assert!(plan.failure.is_some());
    }

    #[test]
    fn failed_completion_requests_fallback() {
        let attempt = RunAttempt::from(&Invocation::manual("job", "run", at(10)));
        let plan = complete_run(
            &attempt,
            &RunOutcome::Failed { exit_code: Some(1) },
            RunTimes {
                started_at: at(11),
                finished_at: at(12),
            },
            "logs/job/run.log".to_string(),
        );

        assert_eq!(plan.record.status, RunStatus::Failed);
        assert!(plan.failure.is_some());
    }
}
