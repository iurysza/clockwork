use std::io::{self, IsTerminal, Write as _};
use std::process::ExitCode;

use chrono::Utc;

use crate::cli::{DefinitionArgs, JobCommands, MutationArgs};
use crate::commands::action_input::{parse_header_lines, parse_method};
use crate::job::definition::{
    CommandAction, JobAction, JobDefinition, PromptAction, WebhookAction,
};
use crate::job::error::JobError;
use crate::job::name::JobName;
use crate::job::plan::{CreateJob, JobOperation, PlannedChange};
use crate::job::service::{JobResult, JobService};

pub fn execute(command: &JobCommands, json: bool) -> ExitCode {
    let result = match command {
        JobCommands::Create {
            name,
            definition,
            mutation,
        } => create(name, definition, mutation, json).map(|()| ExitCode::SUCCESS),
        JobCommands::Status { name } => status(name.as_deref(), json).map(|()| ExitCode::SUCCESS),
        JobCommands::Update {
            name,
            definition,
            mutation,
        } => update(name, definition, mutation, json).map(|()| ExitCode::SUCCESS),
        JobCommands::Enable { name, mutation } => {
            simple_mutation(name, mutation, json, JobOperation::Enable).map(|()| ExitCode::SUCCESS)
        }
        JobCommands::Disable { name, mutation } => {
            simple_mutation(name, mutation, json, JobOperation::Disable).map(|()| ExitCode::SUCCESS)
        }
        JobCommands::Delete { name, mutation } => {
            simple_mutation(name, mutation, json, JobOperation::Delete).map(|()| ExitCode::SUCCESS)
        }
        JobCommands::Trigger { name, mutation } => {
            simple_mutation(name, mutation, json, JobOperation::Trigger).map(|()| ExitCode::SUCCESS)
        }
        JobCommands::Validate { name } => validate(name.as_deref(), json),
        JobCommands::List => list(json).map(|()| ExitCode::SUCCESS),
        JobCommands::History { name, limit } => {
            history(name, *limit, json).map(|()| ExitCode::SUCCESS)
        }
        JobCommands::Logs { name, run, lines } => {
            logs(name, run.as_deref(), *lines, json).map(|()| ExitCode::SUCCESS)
        }
    };

    match result {
        Ok(code) => code,
        Err(error) => {
            present_error(&error, json);
            ExitCode::from(error.exit_code())
        }
    }
}

fn create(
    name: &str,
    input: &DefinitionArgs,
    mutation: &MutationArgs,
    json: bool,
) -> Result<(), JobError> {
    reject_relative_noninteractive_apply(input.schedule.as_deref(), mutation)?;
    let name = JobName::parse(name).map_err(JobError::invalid_input)?;
    let definition = new_definition(name, input)?;
    let operation = JobOperation::Create(CreateJob { definition });
    mutate(&operation, mutation, json)
}

fn validate(name: Option<&str>, json: bool) -> Result<ExitCode, JobError> {
    let name = name
        .map(JobName::parse)
        .transpose()
        .map_err(JobError::invalid_input)?;
    let report = JobService::new().validate(name.as_ref(), Utc::now())?;

    if json {
        println!(
            "{}",
            serde_json::to_string(&report).expect("validation JSON is serializable")
        );
    } else if report.jobs.is_empty() {
        println!("No managed jobs found.");
    } else {
        for job in &report.jobs {
            if job.valid {
                println!("{}: valid", job.job);
            } else {
                println!("{}: invalid", job.job);
                for error in &job.errors {
                    println!("  - {error}");
                }
            }
        }
    }

    Ok(if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn status(name: Option<&str>, json: bool) -> Result<(), JobError> {
    let service = JobService::new();
    let now = Utc::now();
    if let Some(name) = name {
        let name = JobName::parse(name).map_err(JobError::invalid_input)?;
        present_view(&service.inspect(&name, now)?, json);
    } else {
        let views = service.list(now)?;
        if json {
            let jobs: Vec<_> = views.iter().map(view_json).collect();
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({ "ok": true, "jobs": jobs }))
                    .expect("status JSON is serializable")
            );
        } else if views.is_empty() {
            println!("No managed jobs found.");
        } else {
            for view in &views {
                println!("{}: {}", view.name, view.state.label());
            }
        }
    }
    Ok(())
}

fn list(json: bool) -> Result<(), JobError> {
    let views = JobService::new().list(Utc::now())?;
    if json {
        let jobs: Vec<_> = views.iter().map(view_json).collect();
        println!("{}", serde_json::json!({ "ok": true, "jobs": jobs }));
    } else if views.is_empty() {
        println!("No managed jobs found.");
    } else {
        for view in &views {
            println!("{}: {}", view.name, view.state.label());
        }
    }
    Ok(())
}

fn history(name: &str, limit: usize, json: bool) -> Result<(), JobError> {
    let name = JobName::parse(name).map_err(JobError::invalid_input)?;
    // Resolve the managed view first: history is only defined for a job
    // with a managed source, never for stray runtime state.
    JobService::new().inspect(&name, Utc::now())?;
    let records =
        crate::store::history::load_records(Some(name.as_str()), Some(limit)).map_err(|error| {
            JobError::RuntimeFailure {
                message: format!("{error:#}"),
            }
        })?;

    if json {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "job": name.as_str(), "runs": records })
        );
    } else if records.is_empty() {
        println!("No runs recorded for {name}.");
    } else {
        for record in &records {
            println!(
                "{} {} {}",
                record.run_id,
                record.status,
                record.finished_at.to_rfc3339()
            );
        }
    }
    Ok(())
}

fn logs(name: &str, run: Option<&str>, lines: Option<usize>, json: bool) -> Result<(), JobError> {
    let name = JobName::parse(name).map_err(JobError::invalid_input)?;
    // A managed source is required even if its runtime generation has since
    // completed. Logs remain attached to the stable managed job identity.
    JobService::new().inspect(&name, Utc::now())?;
    let output = match run {
        Some(run_id) => crate::engine::logger::read_run_log(name.as_str(), run_id, lines),
        None => crate::engine::logger::read_latest_log(name.as_str(), lines),
    }
    .map_err(|error| JobError::RuntimeFailure {
        message: format!("{error:#}"),
    })?;

    if json {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "job": name.as_str(), "run": run, "log": output })
        );
    } else {
        print!("{output}");
    }
    Ok(())
}

fn present_view(view: &crate::job::state::JobView, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&view_json(view)).expect("status JSON is serializable")
        );
        return;
    }

    println!("Job: {}", view.name);
    println!("State: {}", view.state.label());
    println!("Activation: {}", view.activation);
    println!("Schedule: {}", view.schedule_input);
    println!("Action: {}", view.action_kind);
    println!("Generation: {}", runtime_generation(&view.state));
    if let Some(next_run) = view.state.next_run() {
        println!("Next run: {}", next_run.to_rfc3339());
    }
    println!("Revision: {}", view.revision.combined());
}

fn view_json(view: &crate::job::state::JobView) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "job": view.name.as_str(),
        "state": view.state,
        "activation": view.activation,
        "revision": view.revision.combined(),
        "schedule": view.schedule_input,
        "action": view.action_kind,
        "tags": view.tags,
    })
}

fn runtime_generation(state: &crate::job::state::ManagedJobState) -> u32 {
    match state {
        crate::job::state::ManagedJobState::Draft { .. } => 0,
        crate::job::state::ManagedJobState::Disabled {
            runtime_generation, ..
        }
        | crate::job::state::ManagedJobState::Scheduled {
            runtime_generation, ..
        }
        | crate::job::state::ManagedJobState::Running {
            runtime_generation, ..
        }
        | crate::job::state::ManagedJobState::Completed {
            runtime_generation, ..
        } => *runtime_generation,
    }
}

fn update(
    name: &str,
    patch: &DefinitionArgs,
    mutation: &MutationArgs,
    json: bool,
) -> Result<(), JobError> {
    if !patch.has_changes() {
        return Err(JobError::invalid_input(
            "update needs at least one definition flag",
        ));
    }
    reject_relative_noninteractive_apply(patch.schedule.as_deref(), mutation)?;
    let name = JobName::parse(name).map_err(JobError::invalid_input)?;
    let service = JobService::new();
    let existing = service.definition(&name)?;
    let definition = apply_patch(existing, patch)?;
    mutate(
        &JobOperation::Update(crate::job::plan::UpdateJob { name, definition }),
        mutation,
        json,
    )
}

/// Apply action-scoped patch flags. Action type never changes on update:
/// each branch preserves the previous variant and rejects cross-type flags.
fn apply_action_patch(
    definition: &JobDefinition,
    patch: &DefinitionArgs,
) -> Result<JobAction, JobError> {
    let selected_actions = usize::from(patch.command.is_some())
        + usize::from(patch.prompt.is_some())
        + usize::from(patch.webhook.is_some());
    if selected_actions > 1 {
        return Err(JobError::invalid_input(
            "update accepts at most one action: --command, --prompt, or --webhook",
        ));
    }

    match (&definition.action, selected_actions) {
        (_, 1) if patch.command.is_some() => {
            reject_unrelated(patch, "command")?;
            let JobAction::Command(previous) = &definition.action else {
                return Err(action_change_error(&definition.name));
            };
            Ok(JobAction::Command(CommandAction {
                command: patch.command.clone().unwrap_or_default(),
                shell: patch.shell || previous.shell,
                workdir: patch.workdir.clone().or_else(|| previous.workdir.clone()),
            }))
        }
        (_, 1) if patch.prompt.is_some() => {
            reject_unrelated(patch, "prompt")?;
            let JobAction::Prompt(previous) = &definition.action else {
                return Err(action_change_error(&definition.name));
            };
            Ok(JobAction::Prompt(PromptAction {
                profile: patch.profile.clone().or_else(|| previous.profile.clone()),
                cwd: patch.cwd.clone().or_else(|| previous.cwd.clone()),
                text: patch.prompt.clone().unwrap_or_default(),
            }))
        }
        (_, 1) => {
            reject_unrelated(patch, "webhook")?;
            let JobAction::Webhook(previous) = &definition.action else {
                return Err(action_change_error(&definition.name));
            };
            Ok(JobAction::Webhook(WebhookAction {
                url: patch.webhook.clone().unwrap_or_default(),
                method: patch
                    .method
                    .as_deref()
                    .map(str::parse::<crate::model::action::HttpMethod>)
                    .transpose()
                    .map_err(|error| JobError::invalid_input(error.to_string()))?
                    .unwrap_or(previous.method),
                headers: if patch.header.is_empty() {
                    previous.headers.clone()
                } else {
                    parse_header_lines(&patch.header)
                        .map_err(|error| JobError::invalid_input(error.to_string()))?
                },
                body: patch.body.clone().or_else(|| previous.body.clone()),
            }))
        }
        (_, 0) => match &definition.action {
            JobAction::Command(previous) => {
                reject_unrelated(patch, "command")?;
                Ok(JobAction::Command(CommandAction {
                    command: previous.command.clone(),
                    shell: patch.shell || previous.shell,
                    workdir: patch.workdir.clone().or_else(|| previous.workdir.clone()),
                }))
            }
            JobAction::Prompt(previous) => {
                reject_unrelated(patch, "prompt")?;
                Ok(JobAction::Prompt(PromptAction {
                    profile: patch.profile.clone().or_else(|| previous.profile.clone()),
                    cwd: patch.cwd.clone().or_else(|| previous.cwd.clone()),
                    text: previous.text.clone(),
                }))
            }
            JobAction::Webhook(previous) => {
                reject_unrelated(patch, "webhook")?;
                Ok(JobAction::Webhook(WebhookAction {
                    url: previous.url.clone(),
                    method: patch
                        .method
                        .as_deref()
                        .map(str::parse::<crate::model::action::HttpMethod>)
                        .transpose()
                        .map_err(|error| JobError::invalid_input(error.to_string()))?
                        .unwrap_or(previous.method),
                    headers: if patch.header.is_empty() {
                        previous.headers.clone()
                    } else {
                        parse_header_lines(&patch.header)
                            .map_err(|error| JobError::invalid_input(error.to_string()))?
                    },
                    body: patch.body.clone().or_else(|| previous.body.clone()),
                }))
            }
        },
        (_, _) => unreachable!("action count above one was rejected"),
    }
}

fn apply_patch(
    mut definition: JobDefinition,
    patch: &DefinitionArgs,
) -> Result<JobDefinition, JobError> {
    if let Some(schedule) = &patch.schedule {
        definition.schedule = normalize_schedule(schedule)?;
    }
    if let Some(timeout) = patch.timeout {
        definition.timeout = Some(timeout);
    }
    if !patch.tag.is_empty() {
        definition.tags.clone_from(&patch.tag);
    }
    definition.action = apply_action_patch(&definition, patch)?;
    Ok(definition)
}

fn action_change_error(name: &JobName) -> JobError {
    JobError::illegal_transition(
        name.clone(),
        "managed",
        "update",
        "the action type cannot change on update; delete the job and create it again",
        Some(format!("clockwork job delete {name}")),
    )
}

fn simple_mutation(
    name: &str,
    mutation: &MutationArgs,
    json: bool,
    operation: impl FnOnce(JobName) -> JobOperation,
) -> Result<(), JobError> {
    let name = JobName::parse(name).map_err(JobError::invalid_input)?;
    mutate(&operation(name), mutation, json)
}

fn new_definition(name: JobName, input: &DefinitionArgs) -> Result<JobDefinition, JobError> {
    let schedule = input
        .schedule
        .as_deref()
        .ok_or_else(|| JobError::invalid_input("create requires --schedule <expression>"))
        .and_then(normalize_schedule)?;
    let action_count = usize::from(input.command.is_some())
        + usize::from(input.prompt.is_some())
        + usize::from(input.webhook.is_some());
    if action_count != 1 {
        return Err(JobError::invalid_input(
            "create requires exactly one action: --command, --prompt, or --webhook",
        ));
    }

    let action = if let Some(command) = &input.command {
        reject_unrelated(input, "command")?;
        JobAction::Command(CommandAction {
            command: command.clone(),
            shell: input.shell,
            workdir: input.workdir.clone(),
        })
    } else if let Some(text) = &input.prompt {
        reject_unrelated(input, "prompt")?;
        JobAction::Prompt(PromptAction {
            profile: input.profile.clone(),
            cwd: input.cwd.clone(),
            text: text.clone(),
        })
    } else {
        reject_unrelated(input, "webhook")?;
        JobAction::Webhook(WebhookAction {
            url: input.webhook.clone().unwrap_or_default(),
            method: parse_method(input.method.as_deref())
                .map_err(|error| JobError::invalid_input(error.to_string()))?,
            headers: parse_header_lines(&input.header)
                .map_err(|error| JobError::invalid_input(error.to_string()))?,
            body: input.body.clone(),
        })
    };

    Ok(JobDefinition {
        name,
        schedule,
        action,
        timeout: input.timeout,
        tags: input.tag.clone(),
    })
}

/// Relative one-time inputs are converted at the CLI boundary. A persisted
/// definition must carry an absolute instant, otherwise re-parsing `in 1h`
/// while enabling would silently move the already-reviewed run.
fn reject_relative_noninteractive_apply(
    schedule: Option<&str>,
    mutation: &MutationArgs,
) -> Result<(), JobError> {
    let Some(schedule) = schedule else {
        return Ok(());
    };
    if mutation.yes
        && !mutation.dry_run
        && !io::stdin().is_terminal()
        && matches!(
            crate::schedule::parser::parse_schedule(schedule, Utc::now()),
            Ok(crate::model::schedule::ParsedSchedule::OneShot { .. })
        )
        && chrono::DateTime::parse_from_rfc3339(schedule).is_err()
    {
        return Err(JobError::invalid_input(format!(
            "non-interactive apply cannot repeat relative one-shot schedule '{schedule}'; use the absolute schedule from the dry-run JSON"
        )));
    }
    Ok(())
}

fn normalize_schedule(input: &str) -> Result<String, JobError> {
    let parsed = crate::schedule::parser::parse_schedule(input, Utc::now())
        .map_err(|error| JobError::invalid_input(error.to_string()))?;
    match parsed {
        crate::model::schedule::ParsedSchedule::OneShot { fire_at, .. } => Ok(fire_at.to_rfc3339()),
        crate::model::schedule::ParsedSchedule::RecurringCron { .. }
        | crate::model::schedule::ParsedSchedule::RecurringInterval { .. } => Ok(input.to_string()),
    }
}

fn reject_unrelated(input: &DefinitionArgs, action: &str) -> Result<(), JobError> {
    let invalid = match action {
        "command" => {
            input.profile.is_some()
                || input.cwd.is_some()
                || input.method.is_some()
                || !input.header.is_empty()
                || input.body.is_some()
        }
        "prompt" => {
            input.shell
                || input.workdir.is_some()
                || input.method.is_some()
                || !input.header.is_empty()
                || input.body.is_some()
        }
        "webhook" => {
            input.shell || input.workdir.is_some() || input.profile.is_some() || input.cwd.is_some()
        }
        _ => false,
    };
    if invalid {
        return Err(JobError::invalid_input(format!(
            "one or more flags do not apply to the {action} action"
        )));
    }
    Ok(())
}

fn mutate(operation: &JobOperation, options: &MutationArgs, json: bool) -> Result<(), JobError> {
    let service = JobService::new();
    let now = Utc::now();
    let planned = match options.if_revision.as_deref() {
        Some(revision) if !options.dry_run => service.plan_at_revision(operation, revision, now)?,
        _ => service.plan(operation, now)?,
    };

    if options.dry_run {
        present_plan(&planned, json);
        return Ok(());
    }

    if !confirm(&planned, options, json)? {
        if json {
            println!(
                "{}",
                serde_json::json!({ "ok": true, "changed": false, "cancelled": true })
            );
        }
        return Ok(());
    }
    let expected = options
        .if_revision
        .clone()
        .unwrap_or_else(|| planned.revision.combined());
    let result = service.execute(operation, Some(&expected), Utc::now())?;
    present_result(&result, json);
    Ok(())
}

fn confirm(plan: &PlannedChange, options: &MutationArgs, json: bool) -> Result<bool, JobError> {
    if json {
        if !options.yes || options.if_revision.is_none() {
            return Err(JobError::invalid_input(format!(
                "{} with --json needs --yes and --if-revision {}; first review with --dry-run --json",
                plan.operation,
                plan.revision.combined()
            )));
        }
        return Ok(true);
    }

    if !io::stdin().is_terminal() {
        if !options.yes {
            return Err(JobError::invalid_input(format!(
                "{} needs --yes outside an interactive terminal; first review: clockwork job {} {} --dry-run --json",
                plan.operation, plan.operation, plan.job
            )));
        }
        if options.if_revision.is_none() {
            return Err(JobError::invalid_input(format!(
                "{} needs --if-revision {} outside an interactive terminal",
                plan.operation,
                plan.revision.combined()
            )));
        }
        return Ok(true);
    }

    if options.yes {
        return Ok(true);
    }

    if !json {
        present_plan(plan, false);
    }
    eprint!("Apply this change? [y/N] ");
    io::stderr().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| JobError::RuntimeFailure {
            message: format!("failed to read confirmation: {error}"),
        })?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
        Ok(true)
    } else {
        if !json {
            println!("No changes made.");
        }
        Ok(false)
    }
}

fn present_plan(plan: &PlannedChange, json: bool) {
    if json {
        let value = serde_json::json!({
            "ok": true,
            "operation": plan.operation,
            "job": plan.job.as_str(),
            "changed": false,
            "revision": plan.revision.combined(),
            "current_state": plan.current_state.label(),
            "expected_state": plan.expected_state,
            "changes": plan.changes,
            "schedule": plan.runtime.as_ref().map(|runtime| runtime.schedule_input.as_str()),
            "external_effect": plan.external_effect,
        });
        println!(
            "{}",
            serde_json::to_string(&value).expect("plan JSON is serializable")
        );
        return;
    }

    println!("{} job \"{}\"", title(plan.operation), plan.job);
    println!();
    println!("Current state: {}", plan.current_state.label());
    println!(
        "Requested state: {}",
        plan.expected_state
            .as_ref()
            .map_or("absent", crate::job::state::ManagedJobState::label)
    );
    if plan.changes.is_empty() {
        println!("Planned changes: none");
    } else {
        println!("Planned changes:");
        for change in &plan.changes {
            println!("  - {change}");
        }
    }
    if let Some(runtime) = &plan.runtime {
        println!("Schedule: {}", runtime.schedule_input);
    }
    if let Some(next_run) = plan.next_run {
        println!("Next run: {}", next_run.to_rfc3339());
    }
    println!();
    println!("External effect");
    match &plan.external_effect {
        crate::job::plan::ExternalEffect::None => {
            println!("  None. Nothing runs during this command.");
        }
        crate::job::plan::ExternalEffect::FutureSchedule { next_run, action } => {
            println!("  A {action} action may run at {}.", next_run.to_rfc3339());
            println!("  Nothing runs during this command.");
        }
        crate::job::plan::ExternalEffect::ImmediateTrigger { action } => {
            println!("  A {action} action runs during this command.");
        }
    }
    println!();
    println!("Revision: {}", plan.revision.combined());
    println!("No changes made.");
}

fn present_result(result: &JobResult, json: bool) {
    if json {
        let value = serde_json::json!({
            "ok": true,
            "operation": result.operation,
            "job": result.job.as_str(),
            "changed": result.changed,
            "revision": result.revision.combined(),
            "current_state": result.state.as_ref().map(crate::job::state::ManagedJobState::label),
            "state": result.state,
            "external_effect": result.external_effect,
        });
        println!(
            "{}",
            serde_json::to_string(&value).expect("result JSON is serializable")
        );
        return;
    }

    if result.changed {
        println!("{} job \"{}\"", title(result.operation), result.job);
    } else {
        println!("No changes made.");
    }
    if let Some(state) = &result.state {
        println!("State: {}", state.label());
        if let Some(next_run) = state.next_run() {
            println!("Next run: {}", next_run.to_rfc3339());
        }
    }
}

fn present_error(error: &JobError, json: bool) {
    if json {
        let value = serde_json::json!({
            "ok": false,
            "changed": error.changed(),
            "error": error.to_json(),
        });
        println!(
            "{}",
            serde_json::to_string(&value).expect("error JSON is serializable")
        );
        return;
    }

    eprintln!("error [{}]", error.code());
    eprintln!();
    eprintln!("{error}");
    eprintln!();
    if error.changed() {
        eprintln!("Some changes were made before this failure.");
    } else {
        eprintln!("No changes made.");
    }
    if let Some(recovery) = error.recovery() {
        eprintln!();
        eprintln!("Next action:");
        eprintln!("  {recovery}");
    }
}

fn title(operation: &str) -> String {
    let mut chars = operation.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
