#!/usr/bin/env bash
# Compares this crate's glTF reader against Blender's, file by file.
#
# Blender is an independent implementation, which is the point: our reader
# agreeing with itself proves nothing. The numbers this produced are pinned in
# `crates/m2m-io/tests/glb_read.rs` so CI can check them without Blender.
#
# Usage: tools/glb-blender-diff.sh [directory]   (default: legacy/static)
#
# Takes a few seconds per file — Blender starts a process each time — so expect
# several minutes over the full corpus.
set -uo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
root="${1:-$here/legacy/static}"
blender="${BLENDER:-/Applications/Blender.app/Contents/MacOS/Blender}"

if [ ! -x "$blender" ]; then
    echo "Blender not found at $blender. Set BLENDER=/path/to/blender." >&2
    exit 1
fi

report="$(mktemp -t glb-diff)"
trap 'rm -f "$report"' EXIT
matched=0
differed=0

while IFS= read -r file; do
    mine="$(cargo run --quiet --release -p m2m-io --example read_glb -- "$file" 2>/dev/null)"
    if [ -z "$mine" ]; then
        echo "READ FAIL  $(basename "$file")"
        differed=$((differed + 1))
        continue
    fi
    "$blender" --background --factory-startup \
        --python "$here/tools/blender-fbx-import-check.py" \
        -- "$file" "$report" >/dev/null 2>&1
    if python3 - "$mine" "$file" "$report" <<'PY'
import json, re, sys
mine, path, report = sys.argv[1], sys.argv[2], sys.argv[3]
def ours(key):
    found = re.search(r'"%s": (\d+)' % key, mine)
    return int(found.group(1)) if found else None
try:
    theirs = json.load(open(report))
except Exception as error:
    print(f"BLENDER FAIL  {path}: {error}")
    raise SystemExit(1)
# Blender merges a glTF mesh's primitives into one object, so compare mesh
# counts rather than primitive counts: human-jay.glb is 22 primitives, 1 mesh.
rows = [
    ("meshes", ours("meshes"), theirs.get("meshes")),
    ("bones", ours("bones"), theirs.get("bones")),
    ("vertices", ours("vertices"), sum(theirs.get("mesh_vertices") or [])),
    ("triangles", ours("triangles"), sum(theirs.get("mesh_polygons") or [])),
    ("weighted", ours("weighted_vertices"), theirs.get("weighted_vertices")),
]
bad = [r for r in rows if r[1] != r[2]]
name = path.rsplit("/", 1)[-1]
if bad:
    detail = " ".join(f"{k}: ours={a} blender={b}" for k, a, b in bad)
    print(f"DIFF   {name}  {detail}")
    raise SystemExit(1)
print(f"MATCH  {name}  " + " ".join(f"{k}={a}" for k, a, _ in rows))
PY
    then matched=$((matched + 1)); else differed=$((differed + 1)); fi
done < <(find "$root" -name "*.glb" | sort)

echo "----"
echo "matched: $matched   differed: $differed"
[ "$differed" -eq 0 ]
