# Install Clockwork

Prebuilt releases support Apple silicon and Intel Macs. Linux users can build from source and use the built-in systemd backend or foreground daemon.

## Install the macOS binary

Download and review the release installer, then run it:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://github.com/iurysza/clockwork/releases/latest/download/install.sh
sh install.sh
```

The installer downloads `clockwork-<target>.tar.gz` and checks its SHA-256 checksum against `sha256.sum` before replacing `~/.local/bin/clockwork`.

It does not create jobs, configure agents, or start a scheduler. Existing jobs and profiles remain unchanged.

If `clockwork` is not on your `PATH`, add the binary directory for the current shell:

```sh
export PATH="$HOME/.local/bin:$PATH"
clockwork --version
```

Add the same directory to your shell configuration to keep it available in new terminals.

For a specific release:

```sh
sh install.sh --version 0.3.0
```

## Add the macOS background service

The optional service requires Node.js for installation and checks, and Python 3 for its helper commands. It uses the macOS `launchctl`, `plutil`, and zsh tools.

```sh
sh install.sh --with-service
```

This installs the binary and service files, including `clockwork-service` and a launchd plist. It does not load the service, create jobs, or run actions. An existing service keeps running.

After creating and enabling jobs, start the service:

```sh
clockwork-service start
clockwork-service status
```

See the [service guide](../services/clockwork/README.md) for environment settings, diagnostics, and file locations.

## Run without the optional service

To keep the scheduler in your terminal:

```sh
clockwork daemon
```

It runs until stopped. You do not need Node.js or Python 3 for this mode.

For the built-in OS timer, use one command for your platform:

```sh
# macOS
clockwork setup-backend launchd

# Linux with a systemd user session
clockwork setup-backend systemd
```

These commands install and enable a dispatcher immediately. Review enabled jobs first. Use only one scheduling setup: the optional service, the built-in timer, or the foreground daemon. Do not start competing dispatchers.

## Build from source

From the repository checkout, build with the toolchain pinned in `rust-toolchain.toml`:

```sh
cargo build --locked --release
mkdir -p "$HOME/.local/bin"
install -m 755 target/release/clockwork "$HOME/.local/bin/clockwork"
```

On macOS, preview the service integration before applying it from the checkout:

```sh
node install.mjs
node install.mjs --apply
node install.mjs --doctor
```

The helper link and plist point into this checkout. Keep it in place while using that installation.

## Update an installation

Run the installer again for the latest binary, or pass `--version` for a specific release. If you use the optional service, include `--with-service` to update its helper files too.

An already running daemon continues using its loaded binary. After reviewing enabled jobs and any active runs, restart the optional service with `clockwork-service restart` to use the updated binary.

Next, [create a job](../README.md#schedule-your-first-agent-job) or [configure an agent profile](./agents.md).
