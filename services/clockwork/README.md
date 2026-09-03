# Managed Clockwork service

This optional macOS integration schedules Pi jobs. The default Clockwork install does not add it because it requires Node.js, Pi, helper commands, a job directory, and a launchd plist.

## Install the integration

After you install the Clockwork binary, opt in with:

```sh
sh install.sh --with-pi
```

For development from this checkout, run:

```sh
node install.mjs --apply
```

The installer writes managed files only. It does not load launchd, start a daemon, apply jobs, or run a job action.

## Files it manages

- Job source: `~/.agents/clockwork/jobs.d/`
- Optional environment file: `~/.agents/clockwork/env`
- Runtime state: `~/.local/state/clockwork/`
- Commands: `clockwork-jobs`, `clockwork-pi`, and `clockwork-service`
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

Keep each new job paused. Then validate and preview it before you apply it:

```sh
clockwork-jobs check <job>
clockwork-jobs plan <job>
clockwork-jobs apply <job> --confirm <job> --no-input
clockwork-jobs status <job> --json
```

For a one-time job, create a new job when you need a different scheduled time. Do not edit its time in place.
