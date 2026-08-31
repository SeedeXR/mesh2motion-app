#!/usr/bin/env bash
# Assembles the fuzz corpus from the committed seeds plus any real FBX files
# available. Safe to re-run; cargo-fuzz merges rather than overwrites.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$here/../../.."
for target in fbx_binary fbx_text fbx_pipeline glb; do
    mkdir -p "$here/corpus/$target"
done
# The committed seeds are ASCII, so they go to the text and pipeline targets.
# fbx_binary needs real binary input; it gets the rig below.
cp "$here"/seeds/*.fbx "$here/corpus/fbx_text/"
cp "$here"/seeds/*.fbx "$here/corpus/fbx_pipeline/"
# An `if`, not `[ -f x ] && cp`: under `set -e` a failing test makes that list
# return non-zero and kills the script, so a missing file would look like a
# seeding error rather than an absent optional input.
rig="$repo/legacy/static/test-files/retarget testing/mixamo-original-rig.fbx"
if [ -f "$rig" ]; then
    cp "$rig" "$here/corpus/fbx_binary/"
    cp "$rig" "$here/corpus/fbx_pipeline/"
else
    echo "warning: $rig is missing, so corpus/fbx_binary starts empty and libFuzzer" >&2
    echo "         must discover the 23-byte FBX magic by chance. Restore it." >&2
fi
# The glb target starts from the smallest real files: a rig template (skeleton
# only, ~24K) and a skinned mesh. Big files make libFuzzer slow to mutate
# usefully, so the 5MB animation packs are deliberately left out.
for seed in "$repo/legacy/static/rigs/rig-snake.glb" \
            "$repo/legacy/static/models/model-snake.glb"; do
    if [ -f "$seed" ]; then
        cp "$seed" "$here/corpus/glb/"
    else
        echo "warning: $seed is missing from the glb corpus" >&2
    fi
done

# Gitignored locally, absent in CI.
refs="$repo/references/human_based_fbx_mixamo_animations"
if [ -d "$refs" ]; then
    cp "$refs"/*.fbx "$here/corpus/fbx_binary/" 2>/dev/null || true
    cp "$refs"/*.fbx "$here/corpus/fbx_pipeline/" 2>/dev/null || true
fi
echo "corpus: binary=$(ls "$here/corpus/fbx_binary" | wc -l | tr -d ' ') text=$(ls "$here/corpus/fbx_text" | wc -l | tr -d ' ') pipeline=$(ls "$here/corpus/fbx_pipeline" | wc -l | tr -d ' ') glb=$(ls "$here/corpus/glb" | wc -l | tr -d ' ')"
