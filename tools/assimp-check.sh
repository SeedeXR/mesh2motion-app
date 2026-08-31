#!/usr/bin/env bash
# Compares two model files as the Open Asset Import Library sees them.
#
# Why a third reader. Blender and three.js both read our FBX animation
# perfectly while assimp read *none* of it -- 0 animations and 0 channels where
# the source had 1 and 53 -- because we wrote the null record that terminates a
# node only for nodes that had children. Our own reader cannot see that
# difference at all: it ends a child list at either a null record or the node's
# end offset, so both forms parse to the same document. Two lenient readers
# agreeing is not conformance.
#
# assimp is not Autodesk's FBX SDK, so it is not proof that Maya opens a file.
# It is an independently written importer, which is what makes it useful.
#
# Usage: tools/assimp-check.sh <reference-file> <our-file>
# Exit status is non-zero when a compared field differs.
set -uo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 <reference-file> <our-file>" >&2
    exit 2
fi
if ! command -v assimp >/dev/null; then
    echo "assimp not found. brew install assimp" >&2
    exit 2
fi

# `assimp info` prints two lines starting "Meshes:" -- the count, and the header
# of the per-mesh table. Requiring a digit after the colon keeps only the count;
# matching the key alone silently compared the table header and reported a
# difference between a file and itself.
fields() {
    assimp info "$1" 2>/dev/null |
        grep -E '^(Nodes|Meshes|Animations|Faces|Bones|Animation Channels): +[0-9]+' |
        tr -s ' '
    assimp info "$1" 2>/dev/null | grep -E '^Primitive Types:' | tr -s ' ' | head -1
}

reference="$(fields "$1")"
ours="$(fields "$2")"

if [ -z "$reference" ] || [ -z "$ours" ]; then
    echo "FAIL  assimp could not read one of the files" >&2
    exit 1
fi

status=0
while IFS= read -r line; do
    key="${line%%:*}"
    want="${line#*: }"
    got="$(printf '%s\n' "$ours" | grep -E "^$key:" | sed 's/^[^:]*: //')"
    if [ "$want" = "$got" ]; then
        printf '  OK    %-20s %s\n' "$key" "$want"
    else
        printf '  DIFF  %-20s reference=%s ours=%s\n' "$key" "$want" "$got"
        status=1
    fi
done <<< "$reference"

# Vertices are reported separately: assimp splits vertices at normal and UV
# seams, and our writers carry neither, so a lower count here is the documented
# scope of the writer rather than a defect. Faces are the comparable figure.
printf '  note  vertices %s vs %s (assimp splits on normals/UVs, which we do not write)\n' \
    "$(assimp info "$1" 2>/dev/null | grep -E '^Vertices:' | tr -s ' ' | cut -d' ' -f2)" \
    "$(assimp info "$2" 2>/dev/null | grep -E '^Vertices:' | tr -s ' ' | cut -d' ' -f2)"

exit "$status"
