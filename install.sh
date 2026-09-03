#!/usr/bin/env sh
set -eu

REPOSITORY="iurysza/clockwork"
WITH_PI=0
REQUESTED_VERSION="latest"

usage() {
  cat <<'EOF'
Usage: install.sh [--version VERSION] [--with-pi]

Install the latest Clockwork binary for macOS.

Options:
  --version VERSION  Install an exact release version, such as 0.1.0.
  --with-pi          Also install the optional Pi and launchd integration.
  -h, --help         Show this help.

The default install only adds ~/.local/bin/clockwork. It does not create jobs,
install agent helpers, or start a service. --with-pi writes Pi helper files and
a launchd plist, but still does not load launchd or apply jobs.
EOF
}

fail() {
  printf '%s\n' "error: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -gt 1 ] || fail "--version needs a value"
      REQUESTED_VERSION=$2
      shift 2
      ;;
    --with-pi)
      WITH_PI=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unsupported argument: $1"
      ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || fail "Clockwork releases support macOS only"
case "$(uname -m)" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) fail "Clockwork releases support Apple silicon and Intel Macs only" ;;
esac

for command in awk curl install shasum tar; do
  command -v "$command" >/dev/null 2>&1 || fail "required command missing: $command"
done
if [ "$WITH_PI" -eq 1 ]; then
  command -v node >/dev/null 2>&1 || fail "--with-pi requires Node.js"
fi

case "$REQUESTED_VERSION" in
  latest) RELEASE_PATH="releases/latest/download" ;;
  *)
    VERSION=${REQUESTED_VERSION#v}
    RELEASE_PATH="releases/download/v$VERSION"
    ;;
esac

ARCHIVE="clockwork-$TARGET.tar.gz"
BASE_URL="https://github.com/$REPOSITORY/$RELEASE_PATH"
STAGING=$(mktemp -d "${TMPDIR:-/tmp}/clockwork-install.XXXXXX")
cleanup() { rm -rf "$STAGING"; }
trap cleanup EXIT HUP INT TERM

curl --fail --location --proto '=https' --tlsv1.2 --output "$STAGING/$ARCHIVE" "$BASE_URL/$ARCHIVE"
curl --fail --location --proto '=https' --tlsv1.2 --output "$STAGING/sha256.sum" "$BASE_URL/sha256.sum"

EXPECTED=$(awk -v archive="$ARCHIVE" '$2 == archive || $2 == "*" archive { print $1; exit }' "$STAGING/sha256.sum")
[ -n "$EXPECTED" ] || fail "sha256.sum has no checksum for $ARCHIVE"
ACTUAL=$(shasum -a 256 "$STAGING/$ARCHIVE" | awk '{ print $1 }')
[ "$ACTUAL" = "$EXPECTED" ] || fail "checksum mismatch for $ARCHIVE"

tar -xzf "$STAGING/$ARCHIVE" -C "$STAGING"
BINARY=$(find "$STAGING" -type f -name clockwork -perm -u+x -print -quit)
[ -n "$BINARY" ] || fail "release archive does not contain an executable clockwork binary"
VERSION=$("$BINARY" --version | awk 'NR == 1 { print $2 }')
[ -n "$VERSION" ] || fail "release binary did not report a version"
if [ "$REQUESTED_VERSION" != "latest" ] && [ "$VERSION" != "${REQUESTED_VERSION#v}" ]; then
  fail "release archive reports v$VERSION, expected v${REQUESTED_VERSION#v}"
fi

BIN_DIR="$HOME/.local/bin"
mkdir -p "$BIN_DIR"
TEMPORARY_BINARY="$BIN_DIR/.clockwork.tmp-$$"
install -m 755 "$BINARY" "$TEMPORARY_BINARY"
mv -f "$TEMPORARY_BINARY" "$BIN_DIR/clockwork"
printf '%s\n' "Installed Clockwork v$VERSION to $BIN_DIR/clockwork"

if [ "$WITH_PI" -eq 1 ]; then
  BUNDLE_ROOT=$(dirname "$BINARY")
  [ -f "$BUNDLE_ROOT/install.mjs" ] || fail "release archive does not contain the Pi integration installer"
  [ -d "$BUNDLE_ROOT/services/clockwork" ] || fail "release archive does not contain the Pi integration files"

  RELEASES_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/clockwork/releases"
  BUNDLE_DIR="$RELEASES_DIR/v$VERSION"
  TEMPORARY_BUNDLE="$RELEASES_DIR/.clockwork-v$VERSION.tmp-$$"
  mkdir -p "$RELEASES_DIR"
  if [ ! -d "$BUNDLE_DIR" ]; then
    rm -rf "$TEMPORARY_BUNDLE"
    mkdir "$TEMPORARY_BUNDLE"
    cp -R "$BUNDLE_ROOT/." "$TEMPORARY_BUNDLE"
    mv "$TEMPORARY_BUNDLE" "$BUNDLE_DIR"
  fi
  node "$BUNDLE_DIR/install.mjs" --apply
fi
