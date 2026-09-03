#!/usr/bin/env zsh
emulate -LR zsh
setopt pipefail

ENV_FILE="${CLOCKWORK_ENV_FILE:-$HOME/.agents/clockwork/env}"
MANAGED_STATE_DIR="${CLOCKWORK_STATE_DIR:-$HOME/.local/state/clockwork}"
export CLOCKWORK_HOME="$MANAGED_STATE_DIR"
export CLOCKWORK_BACKEND=none
export TZ=Europe/Berlin
if [[ -f "$ENV_FILE" ]]; then
  set -a
  source "$ENV_FILE"
  set +a
fi
# Fixed managed values cannot be overridden by user configuration.
export CLOCKWORK_HOME="$MANAGED_STATE_DIR"
export CLOCKWORK_BACKEND=none
export TZ=Europe/Berlin
export PATH="$HOME/.local/bin:$HOME/.volta/bin:$HOME/.asdf/shims:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
mkdir -p "$CLOCKWORK_HOME" "$CLOCKWORK_HOME/logs"
chmod 700 "$CLOCKWORK_HOME" "$CLOCKWORK_HOME/logs"
clockwork_bin="${CLOCKWORK_BIN:-$HOME/.local/bin/clockwork}"
[[ -x "$clockwork_bin" ]] || { print -u2 -- "Missing managed clockwork binary: $clockwork_bin"; exit 1; }
exec "$clockwork_bin" daemon
