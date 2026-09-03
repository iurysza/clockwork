use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::engine::action_runner;
use crate::engine::lock::FileLock;
use crate::engine::logger;
use crate::engine::policy::{
    ActionExit, ExecutionAvailability, ExecutionDisposition, RunDecision, RunTimes,
    classify_outcome, complete_run, decide_run,
};
use crate::model::invocation::{Invocation, InvocationSource, RunAttempt};
use crate::model::run_record::{LastRun, RunRecord, RunStatus, Trigger};
use crate::store::config::load_config;
use crate::store::history;
use crate::store::paths;
use crate::store::state;
use crate::util::id::new_run_id;

pub fn execute_invocation(invocation: &Invocation) -> Result<ExecutionDisposition> {
    let job = state::load_job(&invocation.job_id)?
        .with_context(|| format!("Job not found: {}", invocation.job_id))?;

    let observed_at = Utc::now();
    crate::job::inspect::StateInspector::new()
        .verify_managed_runtime(&job, observed_at)
        .map_err(anyhow::Error::from)?;
    if let RunDecision::Ignore(reason) = decide_run(
        &job,
        invocation,
        ExecutionAvailability::Available,
        observed_at,
    ) {
        return Ok(ExecutionDisposition::Ignored(reason));
    }

    let job_lock = FileLock::job_non_blocking(&invocation.job_id)?;
    let availability = if job_lock.is_some() {
        ExecutionAvailability::Available
    } else {
        ExecutionAvailability::Busy
    };

    match decide_run(&job, invocation, availability, observed_at) {
        RunDecision::Start(attempt) => {
            let _job_lock = job_lock.context("available job lock was not retained")?;
            execute_attempt(&job, &attempt)
        }
        RunDecision::Skip(record) => {
            history::append_record(&record)?;
            Ok(ExecutionDisposition::Skipped(record))
        }
        RunDecision::Ignore(reason) => Ok(ExecutionDisposition::Ignored(reason)),
    }
}

fn execute_attempt(
    admitted_job: &crate::model::job::Job,
    attempt: &RunAttempt,
) -> Result<ExecutionDisposition> {
    let (log_file, log_path) = logger::create_log_file(&attempt.job_id, &attempt.run_id)?;
    let started_at = Utc::now();
    let action_result = action_runner::execute(admitted_job, log_file);
    let finished_at = Utc::now();

    let outcome = match action_result {
        Ok(exit) => classify_outcome(Ok(exit)),
        Err(error) => {
            let safe_message = format!("{error:#}");
            append_internal_error(&log_path, &safe_message);
            classify_outcome(Err(safe_message))
        }
    };

    let plan = complete_run(
        attempt,
        &outcome,
        RunTimes {
            started_at,
            finished_at,
        },
        log_path,
    );
    {
        // Record completion through the runtime's narrow scheduler API: it
        // owns claim clearing, run counters, and one-shot completion.
        let _state_lock = FileLock::state()?;
        crate::job::runtime::FsRuntimeStore::complete_run(
            &attempt.job_id,
            &attempt.run_id,
            attempt.recorded_for(),
            matches!(&attempt.source, InvocationSource::Scheduled { .. }),
            LastRun {
                run_id: plan.record.run_id.clone(),
                started_at: plan.record.started_at,
                finished_at: plan.record.finished_at,
                status: plan.record.status,
                exit_code: plan.record.exit_code,
                log_path: plan.record.log_path.clone(),
                error_message: plan.record.error_message.clone(),
            },
        )?;
    }

    history::append_record(&plan.record)?;
    if let Some(failure) = plan.failure {
        let _ = spawn_fallback(
            &failure.job_id,
            &failure.failed_run_id,
            failure.status,
            failure.exit_code,
            &failure.log_path,
            failure.recorded_for,
        );
    }

    Ok(ExecutionDisposition::Completed(outcome))
}

fn append_internal_error(log_path: &str, message: &str) {
    let Ok(home) = paths::clockwork_home() else {
        return;
    };
    let absolute_log = home.join(log_path);
    let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(absolute_log)
    else {
        return;
    };
    writeln!(file, "[clockwork internal error] {message}").ok();
}

/// Append a structured failure line to `~/.clockwork/failures.log`.
fn append_failures_log(
    job_id: &str,
    job_name: &str,
    run_id: &str,
    status: RunStatus,
    exit_code: Option<i32>,
    log_path: &str,
) {
    let Ok(home) = paths::clockwork_home() else {
        return;
    };
    let path = home.join("failures.log");
    let now = Utc::now().to_rfc3339();
    let display_name = if job_name.is_empty() {
        job_id
    } else {
        job_name
    };
    let exit_str = exit_code.map_or_else(String::new, |c| format!(" exit_code={c}"));
    let line = format!(
        "[{now}] FAILED job=\"{display_name}\" id={job_id} run={run_id} status={status}{exit_str} log={log_path}\n"
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// Spawn `clockwork _internal exec-fallback` after a failed invocation.
fn spawn_fallback(
    job_id: &str,
    run_id: &str,
    status: RunStatus,
    exit_code: Option<i32>,
    log_path: &str,
    scheduled_for: DateTime<Utc>,
) -> Result<()> {
    // Always append to failures.log
    let job_name = state::load_job(job_id)?
        .and_then(|j| j.name.clone())
        .unwrap_or_default();
    append_failures_log(job_id, &job_name, run_id, status, exit_code, log_path);

    // Resolve the absolute log path
    let abs_log_path = paths::clockwork_home().map_or_else(
        |_| log_path.to_string(),
        |h| h.join(log_path).to_string_lossy().to_string(),
    );

    // Spawn the private fallback executor as a detached process
    let clockwork_bin =
        std::env::current_exe().context("could not determine clockwork binary path")?;
    let exit_code_str = exit_code.map_or_else(String::new, |c| c.to_string());
    let mut cmd = Command::new(clockwork_bin);
    cmd.args([
        "_internal",
        "exec-fallback",
        job_id,
        "--failed-run-id",
        run_id,
        "--failed-status",
        status.as_str(),
        "--failed-exit-code",
        &exit_code_str,
        "--failed-log-path",
        &abs_log_path,
        "--failed-scheduled-for",
        &scheduled_for.to_rfc3339(),
    ]);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    detach_fallback_process(&mut cmd);
    cmd.spawn()
        .with_context(|| format!("failed to spawn _internal exec-fallback for job {job_id}"))?;
    Ok(())
}

#[cfg(unix)]
fn detach_fallback_process(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_fallback_process(_cmd: &mut Command) {}

/// Execute a fallback command for a failed job. Called from `_internal exec-fallback`.
pub fn exec_fallback(
    job_id: &str,
    failed_run_id: &str,
    failed_status: &str,
    failed_exit_code: &str,
    failed_log_path: &str,
    failed_scheduled_for: &str,
) -> Result<bool> {
    let config = load_config()?;

    // Resolve fallback command: per-job > global config > None
    let job = state::load_job(job_id)?;
    let (fallback_cmd, fallback_shell) = if let Some(ref j) = job {
        if j.on_failure.is_some() {
            (j.on_failure.clone(), j.on_failure_shell)
        } else {
            (config.on_failure.clone(), config.on_failure_shell)
        }
    } else {
        (config.on_failure.clone(), config.on_failure_shell)
    };

    let Some(command) = fallback_cmd else {
        return Ok(true);
    };

    // Concurrency check: count active fallback lock files
    let locks_dir = paths::locks_dir()?;
    let active_fallbacks = std::fs::read_dir(&locks_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("fallback-"))
                })
                .count()
        })
        .unwrap_or(0);

    if active_fallbacks >= usize::try_from(config.max_concurrent_fallbacks).unwrap_or(10) {
        eprintln!(
            "Fallback skipped for job {job_id}: max concurrent fallbacks ({}) reached.",
            config.max_concurrent_fallbacks
        );
        return Ok(true);
    }

    // Acquire a fallback lock
    let fallback_lock_path = locks_dir.join(format!("fallback-{failed_run_id}.lock"));
    let _fallback_lock = FileLock::acquire_non_blocking_path(&fallback_lock_path)?;

    let run_id = new_run_id();
    let (log_file, log_rel_path) = logger::create_log_file(job_id, &run_id)?;
    let started_at = Utc::now();

    // Build stripped environment
    let job_name = job
        .as_ref()
        .and_then(|j| j.name.clone())
        .unwrap_or_default();

    let env_vars = [
        ("PATH", std::env::var("PATH").unwrap_or_default()),
        ("HOME", std::env::var("HOME").unwrap_or_default()),
        ("SHELL", std::env::var("SHELL").unwrap_or_default()),
        ("TERM", std::env::var("TERM").unwrap_or_default()),
        ("CLOCKWORK_FAILED_JOB_ID", job_id.to_string()),
        ("CLOCKWORK_FAILED_JOB_NAME", job_name),
        ("CLOCKWORK_FAILED_RUN_ID", failed_run_id.to_string()),
        ("CLOCKWORK_FAILED_STATUS", failed_status.to_string()),
        ("CLOCKWORK_FAILED_EXIT_CODE", failed_exit_code.to_string()),
        ("CLOCKWORK_FAILED_LOG_PATH", failed_log_path.to_string()),
        (
            "CLOCKWORK_FAILED_SCHEDULED_FOR",
            failed_scheduled_for.to_string(),
        ),
    ];

    // Execute the fallback command
    let result = execute_fallback_command(&command, fallback_shell, &env_vars, log_file);

    let finished_at = Utc::now();
    let (status, exit_code) = match &result {
        Ok((code, timed_out)) => {
            if *timed_out {
                (RunStatus::Timeout, *code)
            } else if code == &Some(0) {
                (RunStatus::Success, *code)
            } else {
                (RunStatus::Failed, *code)
            }
        }
        Err(_) => (RunStatus::InternalError, None),
    };

    // Parse the scheduled_for timestamp for the record
    let scheduled_for_dt = chrono::DateTime::parse_from_rfc3339(failed_scheduled_for)
        .map(|dt| dt.to_utc())
        .unwrap_or(started_at);

    // Record in history
    let record = RunRecord {
        run_id,
        job_id: job_id.to_string(),
        trigger: Trigger::Fallback,
        scheduled_for: scheduled_for_dt,
        started_at,
        finished_at,
        status,
        exit_code,
        log_path: log_rel_path,
        failed_run_id: Some(failed_run_id.to_string()),
        error_message: None,
    };
    history::append_record(&record)?;

    Ok(true)
}

const FALLBACK_TIMEOUT_SECONDS: u64 = 60;

#[allow(clippy::needless_pass_by_value)]
fn execute_fallback_command(
    command: &str,
    shell: bool,
    env_vars: &[(&str, String)],
    log_file: std::fs::File,
) -> Result<(Option<i32>, bool)> {
    let mut cmd = if shell {
        let mut c = Command::new("/bin/sh");
        c.args(["-lc", command]);
        c
    } else {
        let argv = shell_words::split(command)
            .with_context(|| format!("failed to parse fallback command: {command}"))?;
        if argv.is_empty() {
            anyhow::bail!("empty fallback command");
        }
        let mut c = Command::new(&argv[0]);
        if argv.len() > 1 {
            c.args(&argv[1..]);
        }
        c
    };

    // Stripped environment: clear all, then set only what we allow
    cmd.env_clear();
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let stdout_file = log_file.try_clone()?;
    let stderr_file = log_file.try_clone()?;
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));
    action_runner::isolate_child_process(&mut cmd);

    let mut child = cmd.spawn().context("failed to spawn fallback command")?;

    match action_runner::wait_for_child(&mut child, Duration::from_secs(FALLBACK_TIMEOUT_SECONDS))?
    {
        ActionExit::Exited { code } => Ok((code, false)),
        ActionExit::TimedOut => Ok((None, true)),
    }
}
