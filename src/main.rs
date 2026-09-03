mod backend;
mod cli;
mod commands;
mod engine;
mod job;
mod model;
pub mod output;
mod schedule;
mod store;
mod upgrade;
mod util;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Commands, InternalCommands};

#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Job { command } => return commands::job::execute(command, cli.json),

        Commands::Agent { command } => commands::agent::execute(command, cli.json),

        Commands::Setup {
            agent,
            all,
            force,
            dry_run,
            list,
        } => commands::setup::execute(agent.as_deref(), *all, *force, *dry_run, *list, cli.json),

        Commands::Config { key, value } => {
            commands::config::execute(key.as_deref(), value.as_deref(), cli.json)
        }

        Commands::Repair { quiet } => commands::repair::execute(*quiet, cli.json),

        Commands::Doctor => match commands::doctor::execute(cli.json) {
            Ok(code) => return code,
            Err(error) => Err(error),
        },

        Commands::SetupBackend { backend } => commands::setup_backend::execute(backend),

        Commands::Daemon { interval } => commands::daemon::execute(*interval),

        Commands::Upgrade { force } => commands::upgrade::execute(*force, cli.json),

        Commands::Internal { command } => return execute_internal(command),
    };

    if let Some(hint) = upgrade::check::maybe_hint(cli.json) {
        eprintln!("{hint}");
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let exit_code = classify_error(&error);
            eprintln!("{error:#}");
            ExitCode::from(exit_code)
        }
    }
}

fn execute_internal(command: &InternalCommands) -> ExitCode {
    let result = match command {
        InternalCommands::Dispatch => commands::dispatch::execute(),
        InternalCommands::Execute {
            job_id,
            scheduled_for,
            trigger,
            run_id,
        } => match commands::exec::execute(job_id, scheduled_for, trigger, run_id.as_deref()) {
            Ok(true) => return ExitCode::SUCCESS,
            Ok(false) => return ExitCode::from(1),
            Err(error) => Err(error),
        },
        InternalCommands::ExecFallback {
            job_id,
            failed_run_id,
            failed_status,
            failed_exit_code,
            failed_log_path,
            failed_scheduled_for,
        } => {
            let _ = crate::store::paths::ensure_dirs();
            match crate::engine::executor::exec_fallback(
                job_id,
                failed_run_id,
                failed_status,
                failed_exit_code,
                failed_log_path,
                failed_scheduled_for,
            ) {
                Ok(_) => return ExitCode::SUCCESS,
                Err(error) => Err(error),
            }
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(classify_error(&error))
        }
    }
}

/// Map legacy operational-command errors to their documented exit codes.
fn classify_error(error: &anyhow::Error) -> u8 {
    let message = format!("{error:#}");

    if message.contains("Could not parse schedule")
        || message.contains("Invalid cron")
        || message.contains("Empty schedule")
        || message.contains("in the past")
    {
        4
    } else if message.contains("blocked by default")
        || message.contains("HTTP webhooks")
        || message.contains("Only http:// and https://")
    {
        5
    } else if message.contains("No supported scheduling backend")
        || message.contains("Unknown backend")
        || message.contains("lingering is disabled")
    {
        6
    } else if message.contains("Unknown config key")
        || message.contains("exceeds maximum")
        || message.contains("invalid job name")
    {
        2
    } else if message.contains("not found") {
        3
    } else {
        1
    }
}
