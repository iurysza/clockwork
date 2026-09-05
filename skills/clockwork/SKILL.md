---
name: clockwork
description: Schedule recurring or one-time commands, agent prompts, and HTTPS webhooks.
metadata:
  category: agent-workspace
---
# Clockwork

Use `clockwork job` for job changes. New jobs are disabled. Creation and enablement need separate approval.

Job definitions live at `~/.agents/clockwork/jobs.d/<job>/clockwork.yaml`. Runtime state, profiles, history, and logs live under `~/.local/state/clockwork/`. `CLOCKWORK_JOBS_ROOT` and `CLOCKWORK_HOME` override those locations. Never edit runtime files as job configuration.

Read [reference.md](reference.md) for command flags, profiles, and scheduling details.

## Preview, approve, apply

1. Inspect existing jobs and profiles with `clockwork job list --json` and `clockwork agent list --json`.
2. Preview creation with `clockwork job create <job> --schedule <expr> --command <cmd> --dry-run --json`. For other actions, use `--prompt <text> --profile <name>` or `--webhook <url>` instead of `--command`.
3. Show the plan and get approval.
4. Repeat the definition flags with `--yes --if-revision <revision>` to create the disabled job.
5. Preview enablement with `clockwork job enable <job> --dry-run --json` and get separate approval.
6. Enable with `clockwork job enable <job> --yes --if-revision <revision>`.
7. Check `clockwork job status <job> --json`. A job waiting for its next run reports `state.type: scheduled` and a future `state.next_run`. If it is already running or completed, inspect that run instead.

Use the revision from the matching preview. For `in 4h` or another relative one-time schedule, use the preview's absolute `schedule` value when applying. A stale revision changes nothing. Review a fresh plan before retrying.

If status reports an integrity error or inconsistent scheduling data, stop and inspect. Do not trigger a manual run as a workaround. Confirm that no action or external request is in flight before changing scheduling.

## Choose the action

Command jobs execute directly by default. Add `--shell` for shell built-ins, pipes, redirects, or substitutions. Use an absolute `--workdir` for project scripts.

Prompt jobs require a registered profile. Use `clockwork agent detect` for standard profiles or `clockwork agent add` for a custom command. Review detected arguments because some allow unattended tool execution. A job's `--cwd` overrides the profile's working directory. Clockwork passes agent arguments unchanged and does not manage agent sessions.

For background jobs, check the service environment rather than relying on the interactive shell. The optional macOS service reads `~/.agents/clockwork/env` and sets its own `PATH`.

Webhooks require HTTPS unless `allow_insecure_http` is enabled. Headers and bodies are literal stored values, not environment references. Use a command that reads credentials at run time for authenticated requests.

## One-time jobs

Create or update with the final future time, then review enablement. A completed one-time job needs a new future schedule before it can run again. Updating that schedule creates a disabled runtime generation and preserves the name and history.

`completed` includes failure and timeout. Check `state.last_run.status` or history for `success` before reporting success.

## Check the actual result

A successful CLI operation is not proof that the action succeeded or that a provider delivered its result.

1. Inspect the matching run in `clockwork job history <job> --json`.
2. Read the run log and, if applicable, the agent's session or output file.
3. For external effects, check the provider result or the action's delivery record. Make repeatable actions safe against duplicate effects.
4. If the action paused a dependency, confirm that it resumed.

Logs contain action output and may include secrets. Do not share raw prompts, headers, bodies, environment values, or credentials.

## Disable or remove a job

`clockwork job disable <job>` prevents future runs. It does not cancel an action already running.

`trigger` runs an enabled, idle job immediately. Update and delete reject jobs with a run in flight.

Preview deletion with `clockwork job delete <job> --dry-run --json`, get approval, then apply with `--yes --if-revision <revision>`. It removes the runtime job and source directory. Save any needed history first because the history command requires an existing job. Remove an unused profile separately with `clockwork agent rm <name>`.

For service inspection, use `clockwork-service status` and `clockwork-service logs`. The latter prints daemon log paths, not their contents.

## Restore an installation

Before restoring files, stop scheduling and confirm that no action or delivery attempt is active. Restore only the intended binary, plist, or helper files from a known backup. Preserve job definitions and runtime data. Get separate approval before restarting a previous scheduler.
