use anyhow::Result;
use chrono::Utc;

use crate::commands::get::find_job;
use crate::engine::executor;
use crate::model::invocation::Invocation;
use crate::model::job::JobStatus;
use crate::util::id::new_run_id;

pub fn execute(id: &str) -> Result<()> {
    let job = find_job(id)?;

    if job.status == JobStatus::Completed || job.status == JobStatus::Archived {
        anyhow::bail!(
            "Error: Job '{}' ({}) is {} and cannot be run.",
            job.display_name(),
            job.id,
            job.status
        );
    }

    println!(
        "Running job {} ({}) manually...",
        job.display_name(),
        job.id
    );

    let invocation = Invocation::manual(&job.id, new_run_id(), Utc::now());
    let success = executor::execute_invocation(&invocation)?.process_succeeded();

    if success {
        println!("Job completed. Check logs: clockwork logs {}", job.id);
    } else {
        println!(
            "Job encountered an internal error. Check logs: clockwork logs {}",
            job.id
        );
    }

    Ok(())
}
