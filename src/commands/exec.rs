use anyhow::Result;
use chrono::DateTime;

use crate::engine::executor;
use crate::engine::policy::ExecutionDisposition;
use crate::model::invocation::{Invocation, InvocationInputError};
use crate::model::run_record::Trigger;
use crate::store::paths;

pub fn execute(
    job_id: &str,
    scheduled_for: &str,
    trigger: &str,
    run_id: Option<&str>,
) -> Result<bool> {
    paths::ensure_dirs()?;

    let scheduled_for_dt = DateTime::parse_from_rfc3339(scheduled_for)
        .map_err(|e| anyhow::anyhow!("Invalid --scheduled-for timestamp: {e}"))?
        .to_utc();

    let trigger: Trigger = trigger
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid --trigger value: {e}"))?;

    let invocation = match trigger {
        Trigger::Scheduled => Invocation::scheduled(
            job_id,
            run_id.ok_or(InvocationInputError::MissingRunId)?,
            scheduled_for_dt,
        ),
        Trigger::Manual => Invocation::manual(
            job_id,
            run_id.ok_or(InvocationInputError::MissingRunId)?,
            scheduled_for_dt,
        ),
        Trigger::Fallback => return Err(InvocationInputError::FallbackIsNotPrimary.into()),
    };

    let disposition = executor::execute_invocation(&invocation)?;
    Ok(!matches!(
        (trigger, &disposition),
        (Trigger::Manual, ExecutionDisposition::Ignored(_))
    ) && disposition.process_succeeded())
}
