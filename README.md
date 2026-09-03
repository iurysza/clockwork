# Clockwork

![CLOCKWORK wordmark: mauve CLOCK with a wall-clock O, and blue WORK with a cog O](./assets/clockwork-wordmark.png)

> Schedule agent runs and code execution in a predictable way.

Clockwork is a local scheduler for recurring agent work and local commands. Define a job once, choose when it runs, and inspect a clear history of what happened.

## Define a job

Each managed job lives in `~/.agents/clockwork/jobs.d/<job>/`. The directory name and the `name` field in its source must match.

This weekday daily-brief job runs a Pi prompt at 09:00 from Monday to Friday:

```yaml
# ~/.agents/clockwork/jobs.d/daily-brief/clockwork.yaml
name: daily-brief # Must match the job directory.
schedule: "0 9 * * 1-5" # Monday to Friday at 09:00.
action:
  prompt:
    profile: clockwork-pi-daily-brief # Managed Pi runner for this job.
    text: "Write today's daily brief." # Prompt sent to Pi.
timeout: 3600 # Maximum run time in seconds.
```

Definitions describe the schedule and action only. Activation is separate state owned by `clockwork job`; new jobs are disabled until you explicitly enable them.

Pi jobs that use a managed per-job profile also need `pi-profile.json`. Create this file before you run `clockwork job create`. It fixes the working directory, model, thinking level, available tools, and project-file approval behaviour for each scheduled run.

```json
{
  "version": 1,
  "cwd": "~/path/to/project",
  "model": "provider/model",
  "thinking": "high",
  "tools": ["read"],
  "approveProjectFiles": false
}
```

To run code instead, use a command action:

```yaml
action:
  command:
    command: "scripts/refresh-index.sh" # Run a reviewed local command.
```

See the [agent-job template](./services/clockwork/templates/jobs/pi-prompt/) and the [command-job template](./services/clockwork/templates/jobs/command/) for complete examples.

## Schedule useful work


Use Clockwork for work that needs to happen without someone remembering to start it:

- Write a daily brief each weekday morning.
- Ask an agent to pull your Strava data and update you on marathon-training progress every three days.
- Pull data from a source every few hours.

Clockwork gives agent runs and local commands one scheduling model.

## See how a job runs


```mermaid
flowchart LR
    Source[Job source] -->|installs| Job
    Job --> Schedule
    Schedule --> Occurrence[Scheduled time]
    Occurrence --> Invocation
    Trigger[Explicit trigger] --> Invocation
    Invocation --> Decision[Run decision]
    Decision -->|start| Attempt[Run attempt]
    Attempt --> Action[Agent or command]
    Action --> Outcome[Run outcome]
    Outcome --> Record[Run record]
    Decision -->|skip or reject| Record
```

1. You define a job in a managed source.
2. `clockwork job` commands install the source into runtime state, disabled by construction.
3. A scheduled time or explicit trigger creates an invocation.
4. Clockwork starts, skips, or rejects that invocation.
5. Clockwork records the result.

## Run work predictably

- Schedule Pi prompts and local commands from the same job format.
- Preview a job plan before you apply it.
- Keep a history of completed, failed, and skipped runs.
- Disable a job without losing its history.
- Handle missed and overlapping runs with explicit policy.
- Manage recurring and one-time jobs through a clear lifecycle.

## Install on macOS

Clockwork supports Apple silicon and Intel Macs. Download the installer from the latest GitHub Release, inspect it if you want to, then run it:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://github.com/iurysza/clockwork/releases/latest/download/install.sh
sh install.sh
```

The default install verifies the downloaded archive and writes only `~/.local/bin/clockwork`. It does not create jobs, configure an agent, write a launchd plist, or start a service.

### Add the Pi integration

Use the explicit opt-in only when you want Clockwork to schedule Pi jobs. It needs Node.js and Pi because it installs the Pi launcher, job directory, environment file, and an inactive launchd plist.

```sh
sh install.sh --with-pi
```

It still does not load launchd, create jobs, or send external requests.

Create each job disabled, review the plan, and enable it explicitly:

```sh
clockwork job create daily-brief --schedule "0 9 * * 1-5" --prompt "Write the daily brief." --profile clockwork-pi-daily-brief
clockwork job enable daily-brief --dry-run
clockwork job enable daily-brief --yes --if-revision <revision-from-dry-run>
```

Start the service after you have enabled the job:

```sh
clockwork-service start
```

## Manage jobs

Every job is managed. Sources live at `~/.agents/clockwork/jobs.d/<name>/clockwork.yaml`; activation is stored separately and never edited by hand. New jobs are disabled by construction. `enable` is the only command that starts future scheduling.

```text
clockwork job create <name> --schedule <expr> (--command <cmd> | --prompt <text> [--profile <name>] | --webhook <url>)
clockwork job update <name> [definition flags]
clockwork job enable <name>       # The only public activation path.
clockwork job disable <name>
clockwork job trigger <name>      # Explicit immediate run of an enabled job.
clockwork job delete <name>
clockwork job validate [<name>]
clockwork job status [<name>] --json
clockwork job list [--json]
clockwork job history <name> [--json]
clockwork job logs <name> [--json]
```

Mutating commands validate the complete operation, print the planned changes and external-effect classification, and require confirmation. Scripts run the command once with `--dry-run --json`, then repeat it with `--yes --if-revision <revision>`. For a relative one-time schedule such as `in 4h`, use the absolute `schedule` value from the preview when you apply. A stale revision changes nothing. Update and delete refuse to cross a run that is in flight.

## Job basics

| Term | Meaning |
| --- | --- |
| **Job** | A named unit of scheduled work. |
| **Schedule** | The rule that makes a job due. |
| **Invocation** | A request to run a job. |
| **Run decision** | Clockwork's decision to start, skip, or reject an invocation. |
| **Run record** | The saved result of a scheduler run. |

A job moves through this lifecycle:

```mermaid
stateDiagram-v2
    [*] --> Draft: create source
    Draft --> Disabled: install runtime
    Disabled --> Scheduled: enable
    Scheduled --> Disabled: disable
    Scheduled --> Running: scheduler claims run
    Running --> Scheduled: recurring run finishes
    Running --> Completed: one-time run finishes
    Completed --> Disabled: update one-time schedule
    Draft --> [*]: delete
    Disabled --> [*]: delete
    Scheduled --> [*]: delete while idle
    Completed --> [*]: delete
```

## Documentation

- [Managed Clockwork service](./services/clockwork/README.md)
- [Installation reference](./docs/install.md)
- [Release process](./docs/releases.md)

## Development

For a source build, install the binary with Cargo. Add the Pi integration only when you need to test it:

```sh
cargo build --locked --release
install -m 755 target/release/clockwork "$HOME/.local/bin/clockwork"
node install.mjs --apply
```

Run the Rust and integration test suites:

```sh
cargo test --locked
npm test
```

Repository layout:

```text
src/                    Rust scheduler and CLI
services/clockwork/     Managed local service and Pi integration
skills/clockwork/       Agent-facing operating guidance
docs/                   Installation and release references
```
