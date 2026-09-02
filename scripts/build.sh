#!/usr/bin/env bash
# Build the Mesh2Motion desktop app (.app + .dmg) for macOS.
#
# `tauri build` runs the frontend build (beforeBuildCommand: npm run build)
# and bundles the app + dmg targets from tauri.conf.json. If the Apple signing
# env vars are present (see notarize.sh) it also signs and notarizes here;
# otherwise it produces an unsigned build fine for local use.
#
# Usage: scripts/build.sh [extra tauri args]   e.g. --bundles app
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Building Mesh2Motion (tauri build)…"
npx tauri build "$@"

BUNDLE="target/release/bundle"
APP=$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -1 || true)
DMG=$(find "$BUNDLE/dmg"   -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)
echo
echo "Built:"
[ -n "$APP" ] && echo "  app: $APP"
[ -n "$DMG" ] && echo "  dmg: $DMG"
[ -z "$APP$DMG" ] && { echo "  (no artifacts found under $BUNDLE)" >&2; exit 1; }
