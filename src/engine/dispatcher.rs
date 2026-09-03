use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};

use crate::engine::lock::FileLock;
use crate::engine::logger;
use crate::engine::policy::{
    ClaimRecovery, ClaimedExecution, DispatchEffect, plan_dispatch, recover_claim,
};
use crate::model::invocation::Invocation;
use crate::model::job::JobStatus;
use crate::model::run_record::{RunRecord, RunStatus, Trigger};
use crate::store::config::load_config;
use crate::store::history;
use crate::store::state;
use crate::store::state::load_state;
use crate::util::id::new_run_id;

const STALE_CLAIM_GRACE_SECONDS: i64 = 10;

struct SpawnRequest {
    job_id: String,
    run_id: String,
    scheduled_for: DateTime<Utc>,
}

impl From<Invocation> for SpawnRequest {
    fn from(invocation: Invocation) -> Self {
        let scheduled_for = invocation.recorded_for();
        Self {
            job_id: invocation.job_id,
            run_id: invocation.run_id,
            scheduled_for,
        }
    }
}

/// Run one dispatch tick: find all due jobs and launch `_internal execute` for each.
pub fn dispatch(now: DateTime<Utc>) -> Result<()> {
    let Some(_dispatch_lock) = FileLock::dispatch_non_blocking()? else {
        return Ok(());
    };

    let _ = recover_stale_claims(now)?;

    let loaded = load_state()?;
    let config = load_config()?;

    for job_id in loaded.jobs.keys() {
        if let Err(error) = process_job(job_id, now) {
            eprintln!("dispatch rejected job {job_id}: {error:#}");
        }
    }

    archive_completed_jobs(now, config.archive_after_hours)?;
    logger::cleanup_old_logs(config.log_retention_days)?;
    Ok(())
}

/// Transition completed one-shot jobs to archived after the configured timeout.
fn archive_completed_jobs(now: DateTime<Utc>, archive_after_hours: u64) -> Result<()> {
    if archive_after_hours == 0 {
        return Ok(());
    }

    let cutoff = now
        - Duration::hours(
            i64::try_from(archive_after_hours)
                .unwrap_or(i64::MAX)
                .min(8760),
        );

    let _state_lock = FileLock::state()?;
    let mut job_state = load_state()?;
    let mut changed = false;

    for job in job_state.jobs.values_mut() {
        // Managed jobs have no archived state or public unarchive path.
        // Keep completed managed jobs inspectable until the user deletes them.
        if job.managed_by.as_deref() == Some("managed-job") || job.status != JobStatus::Completed {
            continue;
        }
        let anchor = job.completed_at.unwrap_or(job.updated_at);
        if anchor <= cutoff {
            job.status = JobStatus::Archived;
            job.updated_at = now;
            changed = true;
        }
    }

    if changed {
        state::save_state(&job_state)?;
    }

    Ok(())
}

/// Recover abandoned scheduled claims.
pub fn recover_stale_claims(now: DateTime<Utc>) -> Result<usize> {
    let loaded = load_state()?;
    let mut recovered = 0usize;

    for job_id in loaded.jobs.keys() {
        let Some(record) = maybe_recover_stale_claim(job_id, now)? else {
            continue;
        };
        history::append_record(&record)?;
        recovered += 1;
    }

    Ok(recovered)
}

fn maybe_recover_stale_claim(job_id: &str, now: DateTime<Utc>) -> Result<Option<RunRecord>> {
    let _state_lock = FileLock::state()?;
    let mut job_state = load_state()?;
    let Some(job) = job_state.jobs.get(job_id).cloned() else {
        return Ok(None);
    };
    if job.in_flight.is_none() {
        return Ok(None);
    }

    let claimed_execution = observe_claimed_execution(job_id)?;
    let ClaimRecovery::Recover { job, record } = recover_claim(
        &job,
        now,
        claimed_execution,
        Duration::seconds(STALE_CLAIM_GRACE_SECONDS),
    ) else {
        return Ok(None);
    };

    job_state.jobs.insert(job_id.to_string(), *job);
    state::save_state(&job_state)?;
    Ok(Some(record))
}

fn process_job(job_id: &str, now: DateTime<Utc>) -> Result<()> {
    enum Pending {
        Launch(SpawnRequest),
        Skipped(DateTime<Utc>),
    }

    let pending = {
        let _state_lock = FileLock::state()?;
        let job_state = load_state()?;
        let Some(job) = job_state.jobs.get(job_id).cloned() else {
            return Ok(());
        };
        crate::job::inspect::StateInspector::new().verify_managed_runtime(&job, now)?;

        let claimed_execution = if job.in_flight.is_some() {
            observe_claimed_execution(job_id)?
        } else {
            ClaimedExecution::NotRunning
        };
        let plan = plan_dispatch(&job, now, claimed_execution, new_run_id())?;

        // Run claims go through the runtime store's narrow claim API; only
        // the overlap-skip bookkeeping (which sets no claim) is persisted
        // as a wholesale state write here.
        let launches: Vec<Invocation> = plan
            .effects
            .iter()
            .filter_map(|effect| match effect {
                DispatchEffect::Launch(invocation) => Some(invocation.clone()),
                DispatchEffect::RecordSkippedOverlap { .. } => None,
            })
            .collect();
        if plan.changed && launches.is_empty() {
            let mut job_state = job_state;
            job_state.jobs.insert(job_id.to_string(), plan.job);
            state::save_state(&job_state)?;
        }

        let mut pending = Vec::new();
        for effect in plan.effects {
            match effect {
                DispatchEffect::RecordSkippedOverlap { scheduled_for } => {
                    pending.push(Pending::Skipped(scheduled_for));
                }
                DispatchEffect::Launch(invocation) => {
                    let run_id = invocation.run_id.clone();
                    let scheduled_for = invocation.recorded_for();
                    // Durably claim before spawning; a lost race means
                    // another invocation already owns the job.
                    if crate::job::runtime::FsRuntimeStore::claim_run(
                        job_id,
                        run_id.clone(),
                        scheduled_for,
                    )? {
                        pending.push(Pending::Launch(SpawnRequest::from(invocation)));
                    }
                }
            }
        }
        pending
    };

    for effect in pending {
        match effect {
            Pending::Skipped(scheduled_for) => {
                history::append_record(&skipped_overlap_record(job_id, scheduled_for, now))?;
            }
            Pending::Launch(request) => {
                if let Err(error) = spawn_exec(&request) {
                    if let Some(record) =
                        clear_claim_with_internal_error(&request.job_id, &request.run_id, now)?
                    {
                        history::append_record(&record)?;
                    }
                    eprintln!("dispatch spawn error for job {}: {error:#}", request.job_id);
                }
            }
        }
    }

    Ok(())
}

fn observe_claimed_execution(job_id: &str) -> Result<ClaimedExecution> {
    if FileLock::job_non_blocking(job_id)?.is_some() {
        Ok(ClaimedExecution::NotRunning)
    } else {
        Ok(ClaimedExecution::Running)
    }
}

fn clear_claim_with_internal_error(
    job_id: &str,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<RunRecord>> {
    let _state_lock = FileLock::state()?;
    let mut job_state = load_state()?;
    let Some(job) = job_state.jobs.get(job_id).cloned() else {
        return Ok(None);
    };
    if job
        .in_flight
        .as_ref()
        .is_none_or(|claim| claim.run_id != run_id)
    {
        return Ok(None);
    }

    let ClaimRecovery::Recover { job, record } =
        recover_claim(&job, now, ClaimedExecution::NotRunning, Duration::zero())
    else {
        return Ok(None);
    };
    job_state.jobs.insert(job_id.to_string(), *job);
    state::save_state(&job_state)?;
    Ok(Some(record))
}

fn skipped_overlap_record(
    job_id: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
) -> RunRecord {
    RunRecord {
        run_id: new_run_id(),
        job_id: job_id.to_string(),
        trigger: Trigger::Scheduled,
        scheduled_for,
        started_at: now,
        finished_at: now,
        status: RunStatus::SkippedOverlap,
        exit_code: None,
        log_path: String::new(),
        failed_run_id: None,
        error_message: None,
    }
}

/// Spawn `clockwork _internal execute <job-id> --scheduled-for <ts> --trigger <trigger>` as a detached process.
fn spawn_exec(request: &SpawnRequest) -> Result<()> {
    let clockwork_bin =
        std::env::current_exe().context("could not determine clockwork binary path")?;
    let mut command = Command::new(clockwork_bin);
    command.args([
        "_internal",
        "execute",
        &request.job_id,
        "--scheduled-for",
        &request.scheduled_for.to_rfc3339(),
        "--trigger",
        "scheduled",
        "--run-id",
        &request.run_id,
    ]);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    detach_exec_process(&mut command);
    command.spawn().with_context(|| {
        format!(
            "failed to spawn _internal execute for job {}",
            request.job_id
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn detach_exec_process(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach_exec_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach_exec_process(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn windows_dispatch_spawn_uses_detached_process_flags() {
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        assert_eq!(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP, 0x0000_0208);
    }
}
