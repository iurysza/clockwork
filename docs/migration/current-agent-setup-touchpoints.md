# Current agent setup touchpoints

This is a read-only map of the current Schedx installation. It records boundaries, not job prompts, secret values, message bodies, webhook payloads, or generated data.

## Evidence used

- Engine source: `schedx` commit `d1ff3991fbc0562dabb84296e95954a0b038eb0e`.
- Agents integration source: `agents-schedx-production` commit `5064b24`.
- Live paths were inventoried by name only. No current configuration or generated state was changed.

## Clockwork ownership

Clockwork will own these product files when it is installed after separate approval:

- The `clockwork` Rust binary and the managed commands `clockwork-jobs`, `clockwork-pi`, and `clockwork-self-email-once`.
- The managed service helpers and plist template.
- The reproducible pinned build or install receipt for the binary.
- The generated runtime state below `~/.local/state/clockwork/`. This includes runtime jobs, configuration, locks, logs, run history, manifests, backups, ownership records, Pi sessions, and delivery receipts.
- The service label `com.iurysouza.clockwork` and its later plist at `~/Library/LaunchAgents/com.iurysouza.clockwork.plist`.
- The Clockwork skill and paused job templates that this repository ships.

Clockwork does not own an active service until a later cutover explicitly installs and starts it.

## Boundaries that remain outside Clockwork

| Boundary | Current evidence | Future owner | Rule |
| --- | --- | --- | --- |
| Agents profile installer | The agents repository installs service helpers and links managed commands. | agents repository during cutover | Edit only in the later agents change. Do not replace its current integration now. |
| User job source | `~/.agents/schedx/jobs.d/` contains one directory per managed job. | user configuration | Clockwork will later use `~/.agents/clockwork/jobs.d/`. Job source remains user-owned. |
| User environment | `~/.agents/schedx/env` is owner-only. | user configuration and existing secret renderer | Keep credentials out of manifests, service state, receipts, and this repository. |
| Secret loading | Pi uses the existing `@iurysza/pi-secret-env` flow when a scheduled Pi process starts. | Pi and the existing agent secret setup | The scheduler daemon receives no agent secret environment. Raw commands and webhooks need their own least-privilege design. |
| Pi runtime | The existing Pi binary, models, tools, project approval, and secret extension remain managed by Pi and agents. | Pi and agents | Clockwork passes a bounded profile. It does not own Pi configuration. |
| Daily brief skill | The personal vault skill creates the daily brief and any report files. | the vault and its skill | Clockwork only schedules a reviewed prompt job. It never owns the skill or its report content. |
| External providers | Email, WhatsApp, TTS, webhooks, and other delivery systems are reached only by approved job actions. | job owner and provider configuration | No copied job is enabled by this repository. |

## Compile-time and build boundaries

The Rust engine contains the CLI, scheduling model, dispatcher, durable store, backend selection, manifest parser, lock handling, HTTP action support, and test suite. Its package, crate, binary, CLI help, environment names, generated paths, source identifiers, tests, docs, and templates must change from the old product name to Clockwork.

The integration is Node and shell code. It contains:

- `clockwork-jobs`, which validates one source directory per job, validates a limited Pi profile, previews changes in a temporary state copy, tracks only objects it owns, requires an explicit confirmation for apply, and refuses unmanaged collisions.
- `clockwork-pi`, which accepts only a managed job identity and a bounded profile. It derives stable Pi session IDs and state directories instead of accepting raw Pi arguments.
- `clockwork-self-email-once`, which is a specialised delivery guard. It serialises attempts, checks existing delivery evidence, records sanitised receipts, reconciles a lost send response, and blocks automatic retry after ambiguity.
- `clockwork-service`, `launchd-run.sh`, and a plist template. The runner owns a fixed launchd `PATH`, `TZ=Europe/Berlin`, and `CLOCKWORK_BACKEND=none`.

## Install-time boundaries

The proven integration installs an explicitly pinned engine binary, helpers, templates, a managed plist, and command links. It keeps user job directories separate from generated state.

The installer contract is:

- Preview is zero-write. It does not fetch, load launchd, start a daemon, apply jobs, or invoke delivery actions.
- Apply renders or updates only managed files. It does not load launchd or apply user jobs.
- The installer creates missing user directories and an empty environment example. It does not adopt, overwrite, prune, or enable job definitions.
- State and log directories are owner-only.
- An install receipt binds the installed binary to its SHA-256 digest.

## Runtime and service boundaries

A single foreground daemon runs under launchd. The managed runner sets the state directory, disables the engine's built-in backend, sets the timezone, and restores the same fixed values after reading the optional user environment file. It sets a deterministic `PATH` that includes local binaries, Volta, asdf, Homebrew, and system directories.

The service doctor checks helper presence, shell syntax, plist rendering, the fixed runner values, the binary digest when a receipt exists, duplicate daemons, a competing built-in dispatcher, and insecure HTTP configuration. It fails closed when these checks fail.

The current plist has a Schedx-specific agents label. Clockwork must use only `com.iurysouza.clockwork` after cutover. The two services must never run together.

## Config, state, job, session, and receipt boundaries

The current user configuration uses `~/.agents/schedx/`. The current generated state uses `~/.local/state/schedx/`. The state inventory includes engine configuration, jobs, ownership state, logs, locks, run history, manifests, Pi session JSONL files, delivery receipts, backups, and report backups.

The user source directory is authoritative. The ownership map records only runtime objects that the reconciler created. Removing a source directory only removes matching owned runtime objects and profiles. It preserves run history, delivery receipts, and Pi sessions.

New jobs start paused. Apply requires `--confirm <job>` or `--confirm all`. A later enablement changes only `paused`, then repeats validation and planning. A future enabled job must have `status: active` and the expected non-null `next_run`.

One-time jobs need special handling. Do not edit their timestamp in place. If the time changes, pause the old job and create a new job identity. Treat an active job without `next_run` and without run history as blocked. Do not run a manual fallback until no run or delivery attempt is active.

Pi prompt jobs store only a limited `pi-profile.json`. Clockwork derives a stable `clockwork-<job>` session ID and writes the session under `~/.local/state/clockwork/pi-sessions/<job>/`. Prompt text remains in the user-owned job manifest and must not appear in logs, documentation, or receipts.

The specialised daily-brief email guard uses one receipt per Berlin date and a per-date lock. It records status and provider message identity only. It does not store the HTML body or raw provider payload. Ambiguous results block automatic retry.

## Current job and delivery touchpoints

The live configuration contains command, Pi prompt, webhook, recurring, and one-time job definitions. It also contains daily-brief, WhatsApp, TTS, and webhook delivery touchpoints. These names and boundaries are evidence only. This repository does not copy live job definitions, prompts, profiles, sessions, receipts, state, or delivery content.

## Cutover dependency

The later agents-repository change must move the profile installer, helper links, service template, and skill registration to Clockwork. The user must then migrate job sources and selected safe state before the old service stops and before the new service starts. `cutover-plan.md` defines the required zero-double-run order.
