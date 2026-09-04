---
name: clockwork
description: Schedule recurring commands or agent runs.
metadata:
  category: agent-workspace
---
# Clockwork

Use `clockwork job` for every job change. Managed sources live at `~/.agents/clockwork/jobs.d/<job>/clockwork.yaml` and describe the schedule and action only, never activation. Runtime state, history, logs, locks, receipts, profiles, and Pi sessions live under `~/.local/state/clockwork/`. Never edit runtime state as job configuration.

## Safe workflow

1. Preview creation: `clockwork job create <job> --schedule <expr> (--command <cmd> | --prompt <text> --profile <name> [--cwd <dir>] | --webhook <url>) --dry-run --json`.
2. Show the creation plan and get approval.
3. Create the disabled job with the same definition flags plus `--yes --if-revision <revision>`.
4. Preview enablement: `clockwork job enable <job> --dry-run --json`.
5. Show the enablement plan and get separate approval.
6. Enable the job: `clockwork job enable <job> --yes --if-revision <revision>`.
7. Verify with `clockwork job status <job> --json`. An enabled job must report `scheduled` with a future `next_run`.

Use the revision from the matching dry run. For a relative one-time schedule such as `in 4h`, apply the absolute `schedule` value from the preview. A stale revision changes nothing.

Treat `scheduled` with a past or null `next_run` and no run history as blocked. Disable the job. Do not trigger a manual run until you confirm that no run or external effect is in flight.

`trigger` is the only immediate-effect command and requires an enabled, idle job. Update and delete refuse to cross a run that is in flight.

## One-time jobs

A completed one-time schedule is immutable within its runtime generation. To move it, update with a new future schedule: Clockwork replaces the generation, starts it disabled, and keeps the public name and history stable.

- Create or update with the final future time, then enable explicitly.
- Before starting the daemon, require the exact future `next_run` from status.
- After success, require `completed` with a successful last run.

## Actions

### Command

Command jobs use direct argv execution by default. Add `--shell` when the command needs shell built-ins, pipes, redirects, or substitutions. The job owner must make every external effect idempotent. Prefer a Pi prompt job when the action needs skills or several guarded steps.

### Agent prompt

Prompt jobs reference a registered profile from `clockwork agent list`. Run `clockwork agent detect` for standard profiles or `clockwork agent add` for a custom binary, fixed arguments, prompt transport, and cwd. A job-level `--cwd` overrides the profile cwd. Missing profiles and invalid working directories fail before mutation.

For durable Pi work, create one generic profile per job. Pass model, thinking, tools, approval, `--session-id clockwork-<job>`, and `--session-dir ~/.local/state/clockwork/pi-sessions/<job>` as fixed `--arg` values. Clockwork invokes Pi directly. The job references this profile but does not own it.

Scheduled agents start through launchd, so validate the runtime environment instead of relying on the interactive shell.

### HTTPS webhook

Webhooks must use HTTPS unless `allow_insecure_http` is explicitly enabled. Keep headers and credentials out of sources, command output, logs, and receipts.

## Verify each boundary

Do not treat one success signal as proof of the whole workflow.

1. Scheduler: inspect the matching run in `clockwork job history <job> --json`.
2. Agent: inspect the agent session, when used, and the run log under `~/.local/state/clockwork/`.
3. External effect: inspect the job owner's sanitized receipt or provider message ID.
4. Services: confirm one Clockwork daemon and that any paused dependency resumed.

Do not print prompts, webhook headers or bodies, environment values, message bodies, or credentials.

## Inspect and remove

```sh
clockwork job status [<job>] --json
clockwork job history <job> --json
clockwork job logs <job> --json
services/clockwork/service.sh logs
```

Removal is destructive. Preview with `clockwork job delete <job> --dry-run --json`, get approval, then apply with `--yes --if-revision <revision>`. Delete refuses while a run is in flight. It removes the runtime job and source directory. Remove any unneeded agent profile separately with `clockwork agent rm <name>`.

## Rollback

Stop or disable Clockwork first. Confirm that no run or delivery attempt is active. Restore only installer-managed binary, plist, links, and examples from the targeted backup. Preserve `~/.agents/clockwork/` and `~/.local/state/clockwork/`. Get separate approval before reactivating a previous scheduler.
