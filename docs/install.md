# Install on macOS

Clockwork releases support Apple silicon and Intel Macs.

## Install the binary

Download the release installer, inspect it if you want to, then run it:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://github.com/iurysza/clockwork/releases/latest/download/install.sh
sh install.sh
```

The installer downloads the matching `clockwork-<target>.tar.gz` archive and verifies it against `sha256.sum` before it writes `~/.local/bin/clockwork`.

The default install does not create jobs, configure an agent, write a launchd plist, or start a service.

To install a specific version, pass it explicitly:

```sh
sh install.sh --version 0.1.0
```

## Add the background service

The optional service adds a job directory, environment file, service helper, and launchd plist:

```sh
sh install.sh --with-service
```

This requires Node.js. It creates only managed files. It does not load launchd, start a daemon, create jobs, or run an action.

Register prompt agents separately:

```sh
clockwork agent detect
clockwork agent list
```

Detected Pi profiles run `pi --print --mode json`. Use `clockwork agent add` when a job needs fixed model, tool, approval, session, or cwd arguments.

Inspect the native job commands and check the service with:

```sh
clockwork job --help
clockwork-service status
```

`clockwork-service` also supports `start`, `restart`, `stop`, `doctor`, and `logs`.

Mutating job commands support `--dry-run`, `--yes`, `--if-revision`, and `--json`. Non-interactive callers preview with `--dry-run --json` and apply with `--yes --if-revision <revision>`.

## Build from source

For development, build the binary with the pinned Rust toolchain:

```sh
cargo build --locked --release
install -m 755 target/release/clockwork "$HOME/.local/bin/clockwork"
```

To test the service integration from the checkout, run:

```sh
node install.mjs
node install.mjs --apply
node install.mjs --doctor
```
