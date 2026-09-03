#!/usr/bin/env bash
# One-shot: build (signing + notarizing when Apple credentials are set), then
# install Mesh2Motion.app into /Applications. This is the entry point — it runs
# the other scripts for you; you do not need to run build.sh first.
#
# Pass-through args go to the build, e.g. scripts/install.sh --bundles app
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="output"
DEST="/Applications"

# Full Apple credentials present? Then build + sign + notarize + verify;
# otherwise a plain (unsigned, local) build.
signing=0
if { [ -n "${APPLE_SIGNING_IDENTITY:-}" ] || [ -n "${APPLE_CERTIFICATE:-}" ]; } && {
     { [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; } ||
     { [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; }
   }; then
  signing=1
fi

if [ "$signing" = 1 ]; then
  echo "==> Apple credentials found — building, signing and notarizing…"
  bash scripts/notarize.sh "$@"
else
  echo "==> No Apple credentials — building unsigned (fine for local use)…"
  bash scripts/build.sh "$@"
fi

APP=$(find "$OUT" -maxdepth 1 -name '*.app' | head -1 || true)
[ -z "$APP" ] && { echo "error: no .app in ./$OUT after build" >&2; exit 1; }

name=$(basename "$APP")
echo "==> Installing $name into ${DEST}…"
rm -rf "${DEST:?}/$name"
cp -R "$APP" "$DEST/"
echo "Installed $DEST/$name"

# --- The rigging MCP server -------------------------------------------------
# Build the MCP binary (agents drive the whole pipeline through it), verify it
# with its own self-check, and register it with Claude Code if the CLI is here.
echo
echo "==> Building the rigging MCP server (m2m-mcp)…"
cargo build --release -p m2m-mcp
MCP="$PWD/target/release/m2m-mcp"

echo "==> Verifying the MCP server (m2m-mcp --check)…"
"$MCP" --check || { echo "error: MCP self-check failed" >&2; exit 1; }

if command -v claude >/dev/null 2>&1; then
  # -s user: available across all projects, not just this directory.
  # add is idempotent-ish: it errors if the name exists, which is fine.
  if claude mcp add -s user mesh2motion -- "$MCP" >/dev/null 2>&1; then
    echo "Registered the 'mesh2motion' MCP server with Claude Code (user scope)."
  else
    echo "MCP 'mesh2motion' already registered (or add skipped)."
  fi
else
  echo "Claude Code CLI not found — register the MCP manually with:"
  echo "  claude mcp add -s user mesh2motion -- $MCP"
fi
echo "Check the server any time with:  $MCP --check"
