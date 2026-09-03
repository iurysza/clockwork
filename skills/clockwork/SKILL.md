---
name: clockwork
description: Schedule recurring commands or agent runs.
metadata:
  category: agent-workspace
---
# Clockwork

Use `~/.agents/clockwork/jobs.d/<job>/` as the source of truth. Generated runtime state, history, logs, locks, receipts, profiles, and Pi sessions live under `~/.local/state/clockwork/`. Never edit generated state as job configuration.

## Safe workflow

1. Create or edit one job directory. Its directory name, manifest `name`, and only key under `jobs` must match.
2. Set every new job to `paused: true`.
3. Run `clockwork-jobs check <job>` and `clockwork-jobs plan <job>`.
4. Show the source change and plan. Get approval for apply and for any external effect.
5. Apply with `clockwork-jobs apply <job> --confirm <job> --no-input`.
6. Get separate approval for enablement. Change only `paused`, then repeat check, plan, and apply.
7. Run `clockwork-jobs status <job> --json`. An enabled future job must report `status: active` and the expected non-null `next_run`.

Treat `status: active` with `next_run: null` and no run history as blocked. Pause the job. Do not trigger a manual fallback until you confirm that no run or external effect is in flight.

Never use `clockwork add`, `clockwork edit`, or `clockwork rm` for managed jobs.

## One-time jobs

Clockwork cannot safely update an existing one-time schedule. The update stamps `last_scheduled_at`, but the dispatcher runs a one-time job only when that field is empty.

- Create a one-time job with its final future ISO timestamp.
- Apply it paused, then change only `paused` during enablement.
- If the timestamp must change, leave the old job paused and create a fresh job name.
- Before starting the daemon, require the exact future `next_run`.
- After success, require `status: completed`, `last_run_status: success`, and `next_run: null`.

## Actions

### Command

Command jobs use direct argv execution by default. Add `shell: true` when the command needs shell built-ins, semicolons, pipes, redirects, functions, substitutions, or traps. The job owner must make every external effect idempotent. Prefer a Pi prompt job when the action needs skills, generated content, or several guarded steps.

### Pi prompt

Pi prompt jobs add `pi-profile.json`. Allowed settings are cwd, model, thinking, tools, and project-file trust. The launcher derives profile `clockwork-pi-<job>` and stable session `clockwork-<job>`. Callers cannot choose raw Pi arguments or session IDs.

The durable session is stored under `~/.local/state/clockwork/pi-sessions/<job>/`. Scheduled Pi starts through launchd, so validate its runtime environment instead of relying on the interactive shell. Exit code 127 with `env: node: No such file or directory` means the launchd `PATH` is wrong.

### HTTPS webhook

Webhooks must use HTTPS. Keep headers and credentials out of manifests, command output, logs, and receipts. Use only the approved owner-only environment flow.

## Verify each boundary

Do not treat one success signal as proof of the whole workflow.

1. Scheduler: inspect the matching run in `CLOCKWORK_HOME="$HOME/.local/state/clockwork" clockwork history --json`.
2. Agent: inspect the Pi session JSONL and the Clockwork run log under `~/.local/state/clockwork/`.
3. External effect: inspect the job owner's sanitized receipt or provider message ID.
4. Services: confirm one Clockwork daemon and that any paused dependency, such as WhatsApp sync, resumed.

Do not print prompts, webhook headers or bodies, environment values, message bodies, or credentials.

## Inspect and remove

Use:

```sh
clockwork-jobs status [<job>] --json
CLOCKWORK_HOME="$HOME/.local/state/clockwork" clockwork history --json
services/clockwork/service.sh logs
```

Removal is destructive. Get approval, remove the user-owned source directory, run `clockwork-jobs plan`, then run `clockwork-jobs apply --confirm all --no-input`. Reconciliation removes only runtime objects recorded in its ownership map. It preserves history, receipts, and sessions.

## Rollback

Stop or pause Clockwork first. Confirm that no run or delivery attempt is active. Restore only installer-managed binary, plist, links, and examples from the targeted backup. Preserve `~/.agents/clockwork/` and `~/.local/state/clockwork/`. Get separate approval before reactivating a previous scheduler.
