# Clockwork job reference

## Commands

```text
clockwork job create <job> --schedule <expr> (--command <cmd> | --prompt <text> [--profile <name>] [--cwd <dir>] | --webhook <url>) [--timeout <seconds>] [--tag <tag>]...
clockwork job update <job> [definition flags]
clockwork job enable <job>
clockwork job disable <job>
clockwork job trigger <job>
clockwork job delete <job>
clockwork job validate [<job>]
clockwork job status [<job>] --json
clockwork job list --json
clockwork job history <job> [--limit <count>] --json
clockwork job logs <job> [--run <run-id>] [--lines <count>] --json
```

Creation requires one schedule and one action. Update preserves omitted values and cannot change the action type. `--workdir` applies to commands. `--cwd` and `--profile` apply to prompts.

Mutating job commands accept:

```text
--dry-run              Validate and preview without changing state
--yes                  Skip interactive confirmation
--if-revision <value>  Apply only to the inspected revision
--json                 Emit machine-readable output
```

Non-interactive mutations and JSON mutations require both `--yes` and `--if-revision`. Preview with `--dry-run --json` first, then apply after approval. The revision covers the source, runtime job, and referenced profile. A mismatch returns `CW_REVISION_CONFLICT` without applying the operation.

For a relative one-time input such as `in 4h`, replace it with the preview's absolute `schedule` value when applying.

`create` saves a disabled job. `enable` permits future scheduling and requires a future next run. `trigger` runs an enabled, idle job now and waits for it. `update` and `delete` reject in-flight work with `CW_RUN_IN_FLIGHT`.

## Status and results

A single-job status response has `state.type` and `activation`. Only the `scheduled` state has `state.next_run`. List responses have a `jobs` array, and history has a `runs` array.

| State | Meaning |
| --- | --- |
| `draft` | Source exists without a runtime job. |
| `disabled` | Future scheduling is off. |
| `scheduled` | Enabled with a future next run. |
| `running` | Clockwork has claimed a run. The action may still be starting. |
| `completed` | A one-time action finished. Its result may be success, failure, or timeout. |

A trigger's `ok: true` reports a completed CLI operation, not a successful action. Check the matching history record's `status`. Expected results include `success`, `failed`, `timeout`, `internal_error`, and `skipped_overlap`.

Disabling during a run leaves it `running` until it finishes. A recurring job then stays disabled. Updating a completed one-time job with a new future schedule creates a disabled runtime generation while retaining the name and history.

## Schedules

- `0 9 * * MON-FRI` runs Monday to Friday at 09:00 local time. Numeric weekdays use `1` for Sunday through `7` for Saturday.
- `every 30m`, `every 6h`, and `every 3d` expand to cron. They follow calendar boundaries. `every 3d` restarts on day 1 of each month, not every 72 elapsed hours.
- `every 10s` uses an elapsed-time interval.
- `in 4h`, bare `30m`, and future RFC-3339 timestamps define one-time work.

The daemon checks every 10 seconds by default. After downtime, an idle recurring job runs the latest due occurrence rather than replaying every missed run. Due occurrences that overlap a running action are skipped.

## Job files

Each source directory contains `clockwork.yaml`:

```yaml
name: daily-brief
schedule: "0 9 * * MON-FRI"
action:
  prompt:
    profile: pi
    cwd: "~/path/to/project"
    text: "Read this project's notes and write today's brief to daily-brief.md."
timeout: 3600
```

The directory name must match `name`. Sources have no activation field. Do not add `paused` or `enabled`, or edit runtime state by hand. Use `clockwork job update` for definition changes.

Commands run without a shell unless `--shell` is set. Prompt jobs use their named profile or the configured default agent. A job's working directory overrides the profile's directory. Missing profiles and invalid prompt directories block job changes.

Webhook URLs require HTTPS by default. Header and body fields do not expand environment variables. Keep credentials in the executing command's environment rather than in source definitions.

## Agent profiles

```sh
clockwork agent detect
clockwork agent list --json
clockwork agent default pi
```

Detection registers available Pi, Claude, Codex, Gemini, and OpenCode binaries using their absolute paths. Existing profiles remain unchanged unless `--force` is supplied.

Detected arguments are `--print --mode json` for Pi, `-p --enable-auto-mode` for Claude, `exec --full-auto` for Codex, `-p --yolo` for Gemini, and `run` for OpenCode. Review these permission choices before enabling unattended jobs.

From an existing project directory, a custom profile can read prompts through stdin:

```sh
clockwork agent add pi-project \
	--bin "$(command -v pi)" \
	--cwd "$PWD" \
	--prompt-stdin \
	--arg=--print \
	--arg=--mode \
	--arg=json
```

Without `--prompt-stdin`, Clockwork appends the prompt as the final argument. `agent add` replaces a profile with the same name without the job commands' confirmation flow. Jobs share referenced profiles, and deleting a job leaves its profile intact.

Use the agent's own model, tool, and session flags as fixed `--arg` values. Check its installed help for those flags. Clockwork does not create or resume agent sessions.

## Delivery checks

Clockwork records process results and captures output. It cannot prove that a provider delivered a message or completed a remote change. The action must check those effects and prevent duplicates when repeated. Logs may contain sensitive action output.
