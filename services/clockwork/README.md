# Managed Clockwork service

This optional macOS integration lets Clockwork schedule Pi jobs. The default Clockwork install does not add it because it needs Node.js, Pi, helper commands, a job directory, and a launchd plist.

## Install and doctor

After you install the Clockwork binary, opt in with:

```sh
sh install.sh --with-pi
```

The integration writes managed files only. It does not load launchd, start a daemon, apply jobs, or send anything.

For development from this checkout:

```sh
node install.mjs
node install.mjs --apply
node install.mjs --doctor
```

## Ownership

- User config: `~/.agents/clockwork/jobs.d/` and owner-only `~/.agents/clockwork/env`.
- Generated state: `~/.local/state/clockwork/`.
- Managed commands: `clockwork-jobs`, `clockwork-pi`, `clockwork-self-email-once`, and `clockwork-service`.
- Service: `~/Library/LaunchAgents/com.iurysouza.clockwork.plist`.

The installer may create missing config directories and the empty env example. It never adopts, overwrites, or prunes job definitions.

## Lifecycle

After separate approval:

```sh
clockwork-service start
clockwork-service status
clockwork-service restart
clockwork-service stop
clockwork-service doctor
clockwork-service logs
```

The service runs one foreground daemon with `CLOCKWORK_BACKEND=none`, `CLOCKWORK_HOME=~/.local/state/clockwork`, and `TZ=Europe/Berlin`. It never invokes Clockwork's built-in launchd backend.

## Jobs and recovery

Copy a template into a new matching directory, keep `paused: true`, then use `clockwork-jobs check`, `plan`, approved `apply`, and `status`. See the installed `clockwork` skill for approval and removal flows.

For recovery, stop the service, confirm no run is active, and restore only managed files from the installer's targeted backup. Preserve user job files, history, delivery receipts, and sessions. Never activate the old and new daily schedules together.

## One-time job safeguards

Clockwork cannot safely update the schedule of an existing one-time job. Its update path sets `last_scheduled_at` to the update time. The dispatcher runs a one-time job only when `last_scheduled_at` is empty. The job can therefore report `active` but never run.

Use these rules for one-time jobs:

1. Create the job with its final future timestamp. Do not reschedule that job in place.
2. If the time must change, leave the old job paused and create a fresh job name.
3. After enablement, require both `status: active` and a non-null `next_run` that matches the requested time.
4. Treat `status: active` with `next_run: null` and no run history as a blocked job. Pause it before the timestamp passes.
5. Give every external effect its own one-attempt receipt. A scheduler run record does not prove delivery.

A compound `run` command needs `shell: true`. Without it, Clockwork splits the command into arguments and does not interpret shell built-ins, semicolons, functions, redirects, or traps. Prefer a Pi prompt job when the action needs skills, generated content, or several guarded steps.

## Scheduled Pi failures

The launchd runner sets a deterministic `PATH` that includes Volta and Homebrew. Without `~/.volta/bin`, `clockwork-pi` fails with exit code 127 and `env: node: No such file or directory` before Pi starts.

For a failed prompt job, inspect these surfaces in order:

```sh
clockwork-jobs status <job> --json
CLOCKWORK_HOME="$HOME/.local/state/clockwork" clockwork history --json
services/clockwork/service.sh logs
find "$HOME/.local/state/clockwork/logs/<job-id>" -maxdepth 1 -type f -print
```

Pause a recurring sample immediately after its intended attempt. Confirm that no Pi process or external delivery is in flight before running a manual fallback.

## Pi prompt secrets

1Password remains the source of truth for agent API keys. The dotfiles renderer writes the existing owner-only agent environment:

```sh
~/scripts/dev-tools/render-agents-secrets.sh --force
```

Every new Pi process loads that environment through `@iurysza/pi-secret-env`. A Pi prompt started by `clockwork-pi` therefore receives the same keys as an interactive Pi process. The extension blocks secret-file reads and environment dumps, and it redacts values from tool output.

Do not copy keys into a job manifest or `~/.agents/clockwork/env`. Do not make the launchd daemon inherit every agent key. After a key changes, rerun the renderer. The next scheduled Pi process loads the new value; the Clockwork daemon does not need a restart.

This integration applies to Pi prompt jobs. Raw command and webhook jobs do not receive the agent secret environment. Give those job types a separate least-privilege credential design when they need one.
