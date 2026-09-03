#!/usr/bin/env bash
set -euo pipefail
SCRIPT_PATH="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$0")"
SERVICE_DIR="$(dirname "$SCRIPT_PATH")"
ENV_FILE="${CLOCKWORK_ENV_FILE:-$HOME/.agents/clockwork/env}"
STATE_DIR="${CLOCKWORK_HOME:-$HOME/.local/state/clockwork}"
LABEL="${CLOCKWORK_LABEL:-dev.iurysouza.clockwork}"
PLIST="${CLOCKWORK_PLIST:-$HOME/Library/LaunchAgents/$LABEL.plist}"
DOMAIN="gui/$(id -u)"
TARGET="$DOMAIN/$LABEL"
TEMPLATE="$SERVICE_DIR/templates/dev.iurysouza.clockwork.plist.tpl"

usage(){ printf 'Usage: clockwork-service <start|stop|restart|status|doctor|logs>\n'; }
render(){ SERVICE_DIR="$SERVICE_DIR" ENV_FILE="$ENV_FILE" STATE_DIR="$STATE_DIR" LABEL="$LABEL" python3 - "$TEMPLATE" <<'PY'
import os,sys
from pathlib import Path
s=Path(sys.argv[1]).read_text()
for k in ('SERVICE_DIR','ENV_FILE','STATE_DIR','LABEL'): s=s.replace('{{'+k+'}}',os.environ[k])
print(s,end='')
PY
}
loaded(){ launchctl print "$TARGET" >/dev/null 2>&1; }
pid(){ launchctl print "$TARGET" 2>/dev/null | awk -F'= ' '/pid =/ {print $2;exit}' || true; }
daemons(){ pgrep -f '[/]clockwork daemon' 2>/dev/null || true; }
start(){ command -v launchctl >/dev/null; command -v plutil >/dev/null; mkdir -p "$STATE_DIR" "$(dirname "$PLIST")"; chmod 700 "$STATE_DIR"; tmp="$(mktemp "${TMPDIR:-/tmp}/clockwork-plist.XXXXXX")"; render >"$tmp"; plutil -lint "$tmp" >/dev/null; mv "$tmp" "$PLIST"; loaded || launchctl bootstrap "$DOMAIN" "$PLIST"; launchctl enable "$TARGET" >/dev/null 2>&1 || true; launchctl kickstart -k "$TARGET"; status; }
stop(){ if loaded; then launchctl bootout "$TARGET" >/dev/null 2>&1 || launchctl bootout "$DOMAIN" "$PLIST" >/dev/null 2>&1 || true; fi; for _ in $(seq 1 50); do [ -z "$(pid)" ] && break; sleep .2; done; [ -z "$(pid)" ] || { echo 'clockwork daemon did not stop' >&2; return 1; }; }
status(){ printf 'clockwork\n  plist: %s\n  target: %s\n  state: %s\n' "$PLIST" "$TARGET" "$STATE_DIR"; if loaded; then printf '  launchd: loaded pid=%s\n' "$(pid)"; else printf '  launchd: not loaded\n'; fi; printf '  daemons: %s\n' "$(daemons | wc -l | tr -d ' ')"; if [ -x "$HOME/.local/bin/clockwork-jobs" ]; then "$HOME/.local/bin/clockwork-jobs" status --json 2>/dev/null || true; fi; }
doctor(){ rc=0; status || true; for f in "$SERVICE_DIR/launchd-run.sh" "$SERVICE_DIR/clockwork-jobs.mjs" "$SERVICE_DIR/pi-launcher.mjs"; do [ -f "$f" ] || { echo "fail    missing $f"; rc=1; }; done; bash -n "$SERVICE_DIR/service.sh" || rc=1; zsh -n "$SERVICE_DIR/launchd-run.sh" || rc=1; render | plutil -lint - >/dev/null || rc=1; grep -q 'CLOCKWORK_BACKEND=none' "$SERVICE_DIR/launchd-run.sh" || rc=1; if [ -f "$STATE_DIR/install-receipt.json" ] && [ -x "$HOME/.local/bin/clockwork" ]; then node -e 'const fs=require("fs"),c=require("crypto");const r=JSON.parse(fs.readFileSync(process.argv[1]));const h=c.createHash("sha256").update(fs.readFileSync(process.argv[2])).digest("hex");if(r.binarySha256!==h)process.exit(1)' "$STATE_DIR/install-receipt.json" "$HOME/.local/bin/clockwork" || rc=1; fi; count="$(daemons | wc -l | tr -d ' ')"; [ "$count" -le 1 ] || { echo "fail    duplicate clockwork daemons: $count"; rc=1; }; launchctl print "$DOMAIN/com.clockwork.dispatcher" >/dev/null 2>&1 && { echo 'fail    competing built-in clockwork launchd dispatcher'; rc=1; } || true; if [ -f "$STATE_DIR/config.json" ] && grep -Eq '"allow_insecure_http"[[:space:]]*:[[:space:]]*true' "$STATE_DIR/config.json"; then echo 'fail    insecure HTTP enabled'; rc=1; fi; [ "$rc" -eq 0 ] && echo 'ok      clockwork managed service'; return "$rc"; }
logs(){ printf 'stdout: %s/logs/stdout.log\nstderr: %s/logs/stderr.log\nlaunchctl: launchctl print %s\n' "$STATE_DIR" "$STATE_DIR" "$TARGET"; }
case "${1:-}" in start) start;; stop) stop;; restart) stop; start;; status) status;; doctor) doctor;; logs) logs;; *) usage; exit 2;; esac
