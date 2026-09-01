# P2-10b: Verifying an export in Maya

**Status: DONE — verified against the Autodesk FBX SDK and Maya 2027 headless.**
Both are now on the build machine (SDK 2020.3.9, Maya 2027 `mayapy`). The
procedure below is what was run; the results are recorded at the end. `mayapy`
`FBXImport` and the raw SDK (`tools/fbx-sdk-check.sh`) are the reproducible gate;
assimp (`tools/assimp-check.sh`) and Blender remain the CI-side proxies.

**Date:** 2026-09-01 · **Verified with:** Autodesk FBX SDK 2020.3.9 and Maya
2027 (`mayapy` + `fbxmaya`), cross-checked against Blender 5.2.

## Two bugs this found, and the fix

Getting an export to open in Maya specifically (not just Blender) surfaced two
container bugs the lenient readers hid:

1. **Footer id.** The 16-byte footer id was written as zeros; the SDK validates
   it as a CRC over `CreationTime` and `FileId`, so it rejected the file as
   "corrupted". Fixed by writing the fixed triple Blender uses — `FOOTER_ID` in
   `encode.rs` with the matching `CREATION_TIME`/`FILE_ID` in `build.rs`.
2. **`DefaultAttributeIndex`.** Every joint Model and the mesh Model needs this
   property to bind its node attribute (the skeleton attribute, or the geometry).
   Without it Maya imported the transforms but built **0 bones and no mesh
   shape**; Blender was unaffected (`LimbNode` alone is enough there, and it
   computes mesh normals itself). Added to both Models in `build.rs`.

## Why Maya specifically

Autodesk's FBX SDK is the format's reference implementation — the one Maya, Max
and MotionBuilder share. assimp and Blender are independent readers that catch
real conformance bugs (the null-record-after-a-child-list bug in `assimp-check.sh`
is the example), but only the Autodesk SDK proves an export opens in the DCC most
studios actually use. Until this runs, "Maya reads it" is unverified.

## What to export first

Produce one of each format from a known rig — the human is the reference:

```bash
# From the app, or a small harness over mesh2motion_lib::rig::export_{glb,fbx}
# with a bound human, one clip. Or reuse the visual-regression exports.
```

You need: an `.fbx` and a `.glb`, each carrying mesh + skeleton + weights + one
clip.

## The procedure

1. **Import into a clean Maya scene** (`File → Import`, FBX and glTF via the
   Autodesk plugins). It must import with **no error dialog** — an FBX the SDK
   rejects is the failure this whole item exists to catch.
2. **Skeleton:** the joint count matches the export (66 for the human), the
   hierarchy is intact (one root, `pelvis` under it, limbs under the spine), and
   no joint sits at the origin that should not.
3. **Skinning:** the mesh binds to the joints — select the mesh, confirm a
   `skinCluster`, and rotate a joint (e.g. `upperarm_l`); the arm mesh follows and
   nothing detaches or explodes.
4. **Animation:** the clip is present on the timeline, its frame range matches the
   export (e.g. 1.375 s × 24 fps ⇒ frame 33), and playing it moves the rig
   sensibly — the same "nothing flies off" bar the Rust `pose_retarget` test and
   the Blender check apply.
5. **Cross-check the numbers** against what Blender reports for the same file
   (`m2m_bridge::inspect`, or the live bridge): same bones, same mesh vertex count,
   same weighted-vertex total. A disagreement is a real bug, not a Maya quirk.

## Recording the result

Note the Maya version and FBX plugin version, the files checked, and each of the
five checks' outcome, in `handover_session.md`. If a check fails, capture the exact
error text — an FBX SDK rejection message is specific and points at the container
field at fault, which is what the encoder work needs.

## Recorded result (2026-09-01)

A rebuild of `mixamo-original-rig.fbx` through the full semantic layers
(`cargo run -p m2m-io --release --example rebuild_rig`) — mesh + skeleton +
weights + one clip — imported cleanly in all three readers, and the Maya numbers
match the original file exactly:

| Reader | imported | joints/bones | meshes | vertices | skinClusters | anim |
|--------|----------|--------------|--------|----------|--------------|------|
| Autodesk FBX SDK 2020.3.9 | ✅ | 65 | 2 | 24 746 | — | 1 stack, 4.9 s |
| Maya 2027 (`mayapy`/`fbxmaya`) | ✅ | 65 | 4* | 49 492* | 2 | 159 curves |
| Blender 5.2 | ✅ | 65 | 2 | 24 746 (weighted) | — | 1 action |

*Maya reports 4 mesh shapes / 49 492 verts because each skinned mesh carries an
`Orig` intermediate shape — the original file reports the identical numbers, so
this is Maya's skinCluster bookkeeping, not a discrepancy. Root joint
`mixamorig:Hips`, hierarchy intact.

Reproduce: `mayapy` with the script in the scratchpad, or
`tools/fbx-sdk-check.sh <file.fbx>` for the raw SDK.

## CI proxies

`tools/assimp-check.sh <reference> <ours>` and the Blender path (headless + the
live bridge) stay the automated gates on machines without Maya. The SDK/Maya run
above is the one-time Autodesk-SDK confirmation P2-10b existed to get.
