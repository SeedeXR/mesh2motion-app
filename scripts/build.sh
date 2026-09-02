#!/usr/bin/env bash
# Build the Mesh2Motion desktop app (.app + .dmg) for macOS and collect the
# artifacts into ./output.
#
# `tauri build` runs the frontend build (beforeBuildCommand: npm run build) and
# bundles the app + dmg targets from tauri.conf.json. If the Apple signing env
# vars are present (see notarize.sh) it also signs and notarizes here; otherwise
# it produces an unsigned build fine for local use.
#
# Usage: scripts/build.sh [extra tauri args]   e.g. --bundles app
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Building Mesh2Motion (tauri build)…"
npx tauri build "$@"

BUNDLE="target/release/bundle"
OUT="output"
mkdir -p "$OUT"

APP=$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -1 || true)
DMG=$(find "$BUNDLE/dmg"   -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)
[ -z "$APP$DMG" ] && { echo "error: no artifacts under $BUNDLE" >&2; exit 1; }

# Collect into ./output (a signed .app keeps its signature through cp -R).
if [ -n "$APP" ]; then rm -rf "$OUT/$(basename "$APP")"; cp -R "$APP" "$OUT/"; fi
if [ -n "$DMG" ]; then cp -f "$DMG" "$OUT/"; fi

echo
echo "Output (in ./$OUT):"
if [ -n "$APP" ]; then echo "  $(basename "$APP")"; fi
if [ -n "$DMG" ]; then echo "  $(basename "$DMG")"; fi
