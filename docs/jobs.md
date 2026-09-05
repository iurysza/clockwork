# Job reference

A Clockwork job has a name, a schedule, and one action. `clockwork job` manages both its YAML definition and its runtime state. Definitions describe what to run. Separate activation state controls whether the scheduler can run it.

For a first job, follow the [README walkthrough](../README.md#schedule-your-first-agent-job).

## Commands

All commands below start with `clockwork job`.

| Command | Effect |
| --- | --- |
| `create <name> [definition flags]` | Save a new job, disabled. Requires a schedule and one action. |
| `update <name> [definition flags]` | Change a definition while preserving activation. Refuses if a run is in flight. |
| `enable <name>` | Allow future runs. Requires a valid definition and a future next run. |
| `disable <name>` | Prevent future runs. An action already running can finish. |
| `trigger <name>` | Run an enabled, idle job now and wait for its result. |
| `delete <name>` | Remove an idle job and its source directory. Leave agent profiles unchanged. |
| `validate [<name>]` | Check one source definition, or all definitions. |
| `status [<name>]` | Show one job's state, or all job states. |
| `list` | List job names and states. |
| `history <name> [--limit <count>]` | Show recent run records. The default limit is 20. |
| `logs <name> [--run <run-id>] [--lines <count>]` | Read the latest log, or a selected run's log. |

`validate` checks source definitions. `status` also checks that the stored runtime definition matches its source and that a prompt job's profile resolves.

Deletion leaves history and log files on disk, but `history` and `logs` require an existing managed job. Save any records you need before deleting it.

### Definition flags

`create` and `update` accept these flags. An update preserves omitted values.

| Flag | Applies to | Meaning |
| --- | --- | --- |
| `--schedule <expression>` | All actions | [Schedule expression](./scheduling.md#schedule-expressions). Required for creation. |
| `--timeout <seconds>` | All actions | Maximum action duration. New jobs default to `default_timeout_seconds`, initially 300 seconds. |
| `--tag <tag>` | All actions | Repeatable. On update, replaces the full tag list. |
| `--command <command>` | Command | Program and arguments to execute. |
| `--shell` | Command | Run through `/bin/sh -lc` instead of direct execution. |
| `--workdir <directory>` | Command | Working directory. Use an absolute path. |
| `--prompt <text>` | Prompt | Text sent to the agent. |
| `--profile <name>` | Prompt | Registered agent profile. Falls back to the configured default agent if omitted. |
| `--cwd <directory>` | Prompt | Override the profile's working directory. A leading `~` expands to your home directory. |
| `--webhook <url>` | Webhook | HTTPS endpoint. |
| `--method <method>` | Webhook | `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`. Defaults to `POST`. |
| `--header 'Key: Value'` | Webhook | Repeatable request header. On update, replaces the full header list. |
| `--body <text>` | Webhook | Request body for `POST`, `PUT`, or `PATCH`. |

Creation requires exactly one of `--command`, `--prompt`, or `--webhook`. An update cannot change the action type. Delete and recreate the job to switch types.

`--shell` can enable shell execution but cannot turn it off. The CLI has no flag to clear optional fields or empty a tag or header list.

Webhook fields are literal values, with no environment-variable expansion. Keep credentials out of job definitions. For authenticated requests, use a command that reads credentials from its environment at run time.

## Preview and confirmation

`create`, `update`, `enable`, `disable`, `trigger`, and `delete` validate and plan the operation before changing state. In an interactive terminal, they show that plan and ask for confirmation.

| Flag | Meaning |
| --- | --- |
| `--dry-run` | Validate and show the plan without applying it. |
| `--yes` | Skip the interactive confirmation. |
| `--if-revision <revision>` | Apply only if the inspected source, runtime job, and referenced profile still match. |
| `--json` | Emit machine-readable output. Also available on read-only job commands. |

A preview identifies whether the change permits a future run, triggers an action immediately, or has no external effect. It names the action type without printing the command, prompt, or webhook payload.

### Scripts and agents

Outside an interactive terminal, mutations require both `--yes` and `--if-revision`. JSON mutations require both flags even in a terminal.

Capture and review the plan first. This example uses `jq`:

```sh
plan=$(clockwork job enable daily-brief --dry-run --json)
printf '%s\n' "$plan" | jq .
```

After approval, apply the same operation with the reviewed revision:

```sh
revision=$(printf '%s\n' "$plan" | jq -r '.revision')
clockwork job enable daily-brief --yes --if-revision "$revision" --json
clockwork job status daily-brief --json
```

If the source, runtime job, or profile changed, Clockwork returns `CW_REVISION_CONFLICT` without applying the operation. Review a fresh preview before retrying.

For a relative one-time schedule such as `in 4h`, the preview returns an absolute timestamp in `schedule`. Use that timestamp when applying creation or an update. Repeating the relative expression would move the reviewed time, so non-interactive apply rejects it.

### JSON fields

- A preview has `changed: false`, a `revision`, `changes`, an `expected_state`, and an `external_effect`.
- A successful mutation reports `changed` and the resulting `state`. Delete returns a null state.
- A single-job status stores the state name in `state.type`. A scheduled job's next run is in `state.next_run`.
- `list` and status without a name return a `jobs` array. History returns a `runs` array.
- A job error returns `ok: false`, `changed`, and an `error` object with a stable `CW_*` code. The process exits non-zero.

`ok: true` on a trigger means the command completed its operation. Check the run's status in history to determine whether the action succeeded.

## Job files

Clockwork stores each source at `~/.agents/clockwork/jobs.d/<name>/clockwork.yaml`. `CLOCKWORK_JOBS_ROOT` overrides that directory.

A prompt definition looks like this:

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

The directory name must match `name`. Job names start with a letter or digit, contain only letters, digits, `.`, `_`, or `-`, and have at most 64 characters.

The source has no `paused` or `enabled` field. Use `clockwork job update` to change it and `enable` or `disable` to control scheduling. Editing YAML alone does not update the runtime job, and a mismatch blocks normal job operations.

Runtime files live under `~/.local/state/clockwork/`, or `CLOCKWORK_HOME` when set:

| Path | Contents |
| --- | --- |
| `jobs.json` | Runtime jobs, activation, and run claims. |
| `config.json` | Agent profiles and application configuration. |
| `run-history.jsonl` | Run records. |
| `logs/<job>/<run-id>.log` | Captured action output. |
| `locks/` | Scheduler and mutation locks. |

Do not edit runtime files as job configuration.

## Job states

| State | Meaning |
| --- | --- |
| `draft` | A source exists without an installed runtime job, such as after an interrupted create. |
| `disabled` | The scheduler cannot claim new runs. |
| `scheduled` | The job is enabled and has a future `next_run`. |
| `running` | A run has been claimed. The action may be starting or already running. |
| `completed` | A one-time action finished, including failure or timeout. Check `last_run.status` for its result. |

A recurring job returns to `scheduled` after a run, unless you disabled it. Disabling during a run leaves the public state `running` until that run finishes.

To reuse a completed one-time job, update it with a new future schedule, then enable it separately. Clockwork increments `runtime_generation` and keeps the job name and history.

Updates temporarily disable an enabled job, write and verify the new definition, then restore activation. An interrupted update leaves scheduling disabled. Repeat the reviewed operation to recover, inspect the result, and review enablement separately if needed.
