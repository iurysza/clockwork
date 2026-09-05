# macOS background service

The optional Clockwork service runs a daemon through launchd so enabled jobs can run after you close the terminal. The binary-only installer does not add this service.

## Install and start

Follow the [installation guide](../../docs/install.md#add-the-macos-background-service) to install the service files. Installation does not load launchd, create jobs, or register agents.

After reviewing and enabling your jobs, start the service:

```sh
clockwork-service start
clockwork-service status
```

Status reports whether launchd has loaded the service and lists the daemon count. A loaded service does not prove that a job succeeded. Check each job's history and logs.

## Service commands

| Command | Effect |
| --- | --- |
| `clockwork-service start` | Render the plist, load it if needed, and start or restart the daemon. |
| `clockwork-service stop` | Unload the service. |
| `clockwork-service restart` | Stop and start the service. |
| `clockwork-service status` | Show the plist, state directory, launchd status, and daemon count. |
| `clockwork-service doctor` | Check service files, duplicate daemons, competing dispatchers, and selected configuration. |
| `clockwork-service logs` | Print daemon log paths and the launchctl inspection command. |

`logs` prints paths, not log contents. For action output, use `clockwork job logs <name>`.

The helper uses Python 3. Installation and some diagnostic checks also use Node.js.

## Environment

Before starting the daemon, the service runner sources `~/.agents/clockwork/env` as zsh and exports its variables to jobs. This file contains executable shell code, so keep it private and review changes before restarting the service.

Use it for job-specific environment values and credentials. Do not put secrets in tracked files or job definitions. Clockwork does not expand environment variables inside webhook fields.

The runner sets these values after loading the environment file:

- `CLOCKWORK_HOME` points to `~/.local/state/clockwork/` by default.
- `CLOCKWORK_BACKEND=none` prevents the daemon from installing another OS timer.
- `PATH` contains `~/.local/bin`, `~/.volta/bin`, `~/.asdf/shims`, `/opt/homebrew/bin`, `/usr/local/bin`, and the macOS system directories.

The environment file cannot override those fixed values. Use absolute paths for commands outside that `PATH`.

The service daemon checks for due jobs every 10 seconds. Run only one scheduler setup. Do not also enable the built-in `com.clockwork.dispatcher` launchd timer.

## Installed files

| Path | Purpose |
| --- | --- |
| `~/.local/bin/clockwork-service` | Link to the service helper. |
| `~/Library/LaunchAgents/dev.iurysouza.clockwork.plist` | launchd service definition. |
| `~/.agents/clockwork/jobs.d/` | Job sources managed by `clockwork job`. |
| `~/.agents/clockwork/env` | User-owned environment file, preserved on reinstall. |
| `~/.local/state/clockwork/` | Runtime state, profiles, history, locks, and logs. |
| `~/.local/share/clockwork/releases/v<version>/` | Release bundle containing the service files. Uses `XDG_DATA_HOME` when set. |

Daemon stdout and stderr go to `logs/stdout.log` and `logs/stderr.log` under the state directory. Action logs live separately under `logs/<job>/`.

The installer never changes existing job definitions or agent profiles.

## Job control

Use [`clockwork job`](../../docs/jobs.md) to create, enable, disable, or remove jobs. Disabling a job stops future scheduling without deleting its history. An action already running can finish.

Stopping the service is not proof that every action or external request has stopped. Before restarting or restoring files, inspect active runs and any external effects they may have started.

Use [`clockwork agent`](../../docs/agents.md) for profiles. Jobs reference profiles but do not own them.
