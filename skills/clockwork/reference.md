# Clockwork managed job reference

All public jobs are managed. The source at `~/.agents/clockwork/jobs.d/<job>/clockwork.yaml` describes the schedule and action; activation is separate operational state owned by the CLI.

## Commands

```text
clockwork job create <job> --schedule <expr> (--command <cmd> | --prompt <text> [--profile <name>] [--cwd <dir>] | --webhook <url>) [--timeout <s>] [--tag <t>]...
clockwork job update <job> [definition flags]
clockwork job enable <job>
clockwork job disable <job>
clockwork job trigger <job>
clockwork job delete <job>
clockwork job validate [<job>]
clockwork job status [<job>] --json
clockwork job list --json
clockwork job history <job> --json
clockwork job logs <job> --json
```

Every mutating command validates the complete operation before mutation and supports:

```text
--dry-run              Validate and preview without changing state
--yes                  Skip the interactive confirmation
--if-revision <value>  Apply only to the inspected revision
--json                 Stable machine-readable output
```

Non-interactive flow: run with `--dry-run --json`, record `revision`, then repeat with `--yes --if-revision <revision>`. For a relative one-time schedule such as `in 4h`, replace it with the preview's absolute `schedule` value when you apply. The command revalidates everything; a stale revision is a `CW_REVISION_CONFLICT` and changes nothing. The revision covers the managed source, runtime job, and resolved referenced profile.

`create` always produces a disabled job. `enable` is the only public activation path and requires a future `next_run`. `trigger` requires an enabled, idle job and never acts as an automatic fallback. `update` and `delete` refuse to cross an in-flight run (`CW_RUN_IN_FLIGHT`).

## Job files

Each source job directory contains `clockwork.yaml`:

```yaml
name: daily-brief
schedule: "0 9 * * 1-5"
action:
  prompt:
    profile: pi-daily-brief
    cwd: "~/path/to/project"
    text: "Write today's daily brief."
timeout: 3600
```

The directory name and the `name` field must match. The definition has no activation field; do not add `paused` or `enabled`.

A prompt job uses its named profile or the configured default agent. A profile may define a cwd. The prompt action may override it. A missing profile or invalid effective cwd fails closed. Jobs never own profiles, so deletion leaves profiles unchanged. A command job runs direct arguments unless `shell: true` is set in the source. A webhook job must use HTTPS.

## Agent profiles

Detect standard agents with `clockwork agent detect`. It registers Pi, Claude, Codex, Gemini, and OpenCode when their binaries are on `PATH`. OpenCode runs through `opencode run`.

Use `clockwork agent add` for fixed settings:

```sh
clockwork agent add pi-daily-brief \
  --bin "$(command -v pi)" \
  --cwd "~/path/to/project" \
  --prompt-stdin \
  --arg=--print \
  --arg=--mode \
  --arg=json \
  --arg=--model \
  --arg=provider/model \
  --arg=--thinking \
  --arg=high \
  --arg=--tools \
  --arg=read,bash,write \
  --arg=--approve \
  --arg=--session-id \
  --arg=clockwork-daily-brief \
  --arg=--session-dir \
  --arg="$HOME/.local/state/clockwork/pi-sessions/daily-brief"
```

Use one profile per job when Pi needs a stable durable session. Use `--no-approve` instead of `--approve` when Pi must ignore project-local files. Other agents use the same `--bin`, `--arg`, `--prompt-stdin`, and `--cwd` fields with their own CLI flags.

## One-time jobs

A completed one-time schedule is immutable within its generation. Updating the schedule replaces the generation, starts it disabled, and keeps the public name and history stable. Do not edit the source by hand. Use `clockwork job update <job> --schedule <future-time>` or create a new job identity. Enable only after status reports the exact future `next_run`.

## Delivery receipts

A scheduler run proves only that Clockwork started the action. It does not prove provider delivery. Keep idempotent receipts in the action that performs any external delivery.
