# Managed Clockwork service

This optional macOS integration schedules Pi jobs. The default Clockwork install does not add it because it requires Node.js, Pi, a job directory, and a launchd plist.

## Install the integration

After you install the Clockwork binary, opt in with:

```sh
sh install.sh --with-pi
```

For development from this checkout, run:

```sh
node install.mjs --apply
```

The installer writes managed files only. It does not load launchd, start a daemon, create jobs, or run a job action.

## Files it manages

- Job source: `~/.agents/clockwork/jobs.d/`
- Optional environment file: `~/.agents/clockwork/env`
- Runtime state: `~/.local/state/clockwork/`
- Commands: `clockwork-pi` and `clockwork-service` (jobs are managed by `clockwork job`)
- launchd plist: `~/Library/LaunchAgents/dev.iurysouza.clockwork.plist`

The installer never adopts, overwrites, or removes job definitions.

## Start the service

After you review your jobs, run one of these commands:

```sh
clockwork-service start
clockwork-service status
clockwork-service restart
clockwork-service stop
clockwork-service doctor
clockwork-service logs
```

The service runs one foreground daemon. It sets `CLOCKWORK_BACKEND=none` and stores runtime state in `~/.local/state/clockwork/`.

## Manage jobs

Jobs are created disabled and enabled explicitly. For a managed per-job Pi profile, write `pi-profile.json` in the job directory before `create`:

```sh
clockwork job create <job> --schedule "0 9 * * 1-5" --prompt "..." --profile clockwork-pi-<job>
clockwork job enable <job> --dry-run --json
clockwork job enable <job> --yes --if-revision <revision>
clockwork job status <job> --json
```

A prompt job with a `pi-profile.json` companion owns the derived profile `clockwork-pi-<job>`. Create and update maintain it automatically. Delete removes it when no other job uses it. To change a completed one-time job, use `clockwork job update <job> --schedule <future-time>`; Clockwork creates a new disabled runtime generation.
