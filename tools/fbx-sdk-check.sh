#!/usr/bin/env bash
# Compiles (once, cached) and runs the FBX SDK import checker on a file (P2-10b).
set -euo pipefail
SDK="/Applications/Autodesk/FBX SDK/2020.3.9"
SRC="$(dirname "$0")/fbx-sdk-check.cpp"
BIN="/tmp/m2m-fbx-sdk-check"
if [ ! -x "$BIN" ] || [ "$SRC" -nt "$BIN" ]; then
    clang++ -std=c++17 -O2 "$SRC" -o "$BIN" \
        -I"$SDK/include" \
        "$SDK/lib/clang/release/libfbxsdk.a" \
        -lz -lxml2 -liconv \
        -framework CoreFoundation -framework SystemConfiguration >&2
fi
"$BIN" "$@"
