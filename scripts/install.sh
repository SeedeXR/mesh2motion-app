#!/usr/bin/env bash
# Install the built Mesh2Motion.app into /Applications.
# Uses the built .app directly; falls back to mounting the .dmg if only that
# exists (e.g. a downloaded release). Run scripts/build.sh first for a local build.
set -euo pipefail
cd "$(dirname "$0")/.."

BUNDLE="target/release/bundle"
DEST="/Applications"

install_app() {  # $1 = path to .app
  local app="$1" name; name=$(basename "$app")
  rm -rf "${DEST:?}/$name"
  cp -R "$app" "$DEST/"
  echo "Installed $DEST/$name"
}

APP=$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -1 || true)
if [ -n "$APP" ]; then
  install_app "$APP"
  exit 0
fi

DMG=$(find "$BUNDLE/dmg" -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)
[ -z "$DMG" ] && { echo "error: no .app or .dmg found — run scripts/build.sh first" >&2; exit 1; }
echo "==> Mounting $DMG…"
MNT=$(hdiutil attach "$DMG" -nobrowse -readonly | awk -F'\t' '/\/Volumes\//{print $NF; exit}')
trap '[ -n "${MNT:-}" ] && hdiutil detach "$MNT" -quiet || true' EXIT
install_app "$(find "$MNT" -maxdepth 1 -name '*.app' | head -1)"
