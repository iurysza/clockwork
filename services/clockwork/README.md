# Managed Clockwork service

This optional macOS integration runs Clockwork schedules in the background. The default Clockwork install adds only the binary.

## Install the integration

After installing Clockwork, opt in with:

```sh
sh install.sh --with-service
```

For development from this checkout, run:

```sh
node install.mjs --apply
```

The installer writes managed files only. It does not load launchd, start a daemon, create jobs, register agents, or run an action.

## Files it manages

- Job sources: `~/.agents/clockwork/jobs.d/`
- Optional environment file: `~/.agents/clockwork/env`
- Runtime state: `~/.local/state/clockwork/`
- Command: `clockwork-service`
- launchd plist: `~/Library/LaunchAgents/dev.iurysouza.clockwork.plist`

The installer never changes job definitions or agent profiles.

## Start the service

After reviewing and enabling your jobs, run one of these commands:

```sh
clockwork-service start
clockwork-service status
clockwork-service restart
clockwork-service stop
clockwork-service doctor
clockwork-service logs
```

The service runs one foreground daemon. It sets `CLOCKWORK_BACKEND=none` and stores runtime state in `~/.local/state/clockwork/`.

## Manage jobs and agents

Register agents through the generic profile commands:

```sh
clockwork agent detect
clockwork agent add <name> --bin <path> [--cwd <dir>] [--arg <value>]...
clockwork agent list
```

Jobs are created disabled and enabled separately:

```sh
clockwork job create <job> --schedule "0 9 * * 1-5" --prompt "..." --profile <name> [--cwd <dir>]
clockwork job enable <job> --dry-run --json
clockwork job enable <job> --yes --if-revision <revision>
clockwork job status <job> --json
```

A job-level cwd overrides the profile cwd. Jobs reference profiles but never create, update, or delete them. To change a completed one-time job, run `clockwork job update <job> --schedule <future-time>`. Clockwork creates a new disabled runtime generation.
