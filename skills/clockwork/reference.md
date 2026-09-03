# Clockwork managed job reference

Use the managed integration for jobs below `~/.agents/clockwork/jobs.d/`. The engine's direct mutating commands do not own these jobs.

## Commands

```sh
clockwork-jobs check <job>
clockwork-jobs plan <job>
clockwork-jobs apply <job> --confirm <job> --no-input
clockwork-jobs status <job> --json
CLOCKWORK_HOME="$HOME/.local/state/clockwork" clockwork history --json
```

`check` and `plan` do not modify the user job source or runtime state. `apply` requires an exact confirmation. New source jobs must set `paused: true` on their first apply.

## Job files

Each source job directory contains `clockwork.yaml`. Its directory name, top-level `name`, and only key below `jobs` must match.

A Pi prompt job also contains `pi-profile.json`. The launcher accepts only `cwd`, `model`, `thinking`, `tools`, and `approveProjectFiles`. It derives the agent profile and session ID from the job identity.

A command job runs direct arguments unless `shell: true` is set. A webhook job must use HTTPS.

## One-time jobs

Create a one-time job with its final future time. Do not edit that time later. To change it, leave the old job paused and create a new job identity. Enable only after status reports the exact future `next_run`.

## Delivery receipts

A scheduler run proves only that Clockwork started the action. It does not prove provider delivery. Keep an idempotent receipt for each delivery attempt. The bundled daily-brief email guard records a sanitised receipt and blocks automatic retry after an ambiguous result.
