#!/usr/bin/env bash
# Produce a SIGNED + NOTARIZED release build and verify it.
#
# Tauri v2 signs and notarizes during `tauri build` when the Apple credentials
# are in the environment. This script checks they are set, runs the build, then
# verifies the signature and stapled notarization ticket with Apple's own tools.
#
# Signing identity (one of):
#   APPLE_SIGNING_IDENTITY   e.g. "Developer ID Application: Name (TEAMID)"
#   APPLE_CERTIFICATE + APPLE_CERTIFICATE_PASSWORD   base64 .p12 + its password (CI)
#
# Notarization credentials (one method):
#   Apple ID:  APPLE_ID  APPLE_PASSWORD (app-specific)  APPLE_TEAM_ID
#   API key:   APPLE_API_ISSUER  APPLE_API_KEY  APPLE_API_KEY_PATH
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ] && [ -z "${APPLE_CERTIFICATE:-}" ]; then
  echo "error: set APPLE_SIGNING_IDENTITY (or APPLE_CERTIFICATE + APPLE_CERTIFICATE_PASSWORD)" >&2
  exit 1
fi
have_appleid=$([ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && echo 1 || echo 0)
have_apikey=$([ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ] && echo 1 || echo 0)
if [ "$have_appleid" = 0 ] && [ "$have_apikey" = 0 ]; then
  echo "error: set notarization credentials — either" >&2
  echo "       APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID, or" >&2
  echo "       APPLE_API_ISSUER + APPLE_API_KEY + APPLE_API_KEY_PATH" >&2
  exit 1
fi

bash scripts/build.sh "$@"

BUNDLE="target/release/bundle"
APP=$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' | head -1)
DMG=$(find "$BUNDLE/dmg"   -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)

echo "==> Verifying signature and notarization…"
codesign --verify --deep --strict --verbose=2 "$APP"
spctl --assess --type execute --verbose=4 "$APP"
xcrun stapler validate "$APP"
[ -n "$DMG" ] && xcrun stapler validate "$DMG"
echo "OK: signed, notarized, stapled."
