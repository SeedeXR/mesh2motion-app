# P2-10b: Verifying an export in Maya

**Status: procedure ready; the run needs Maya.** Maya / the Autodesk FBX SDK is
not on the build machine, so this cannot be executed here — assimp
(`tools/assimp-check.sh`) is the independent-reader proxy used instead (P2-10).
This is the exact procedure to run on a machine that *does* have Maya, and what
"passes" means, so the check is reproducible rather than ad hoc.

**Date:** 2026-09-01 · **Proxy in use:** assimp (an independent importer, but not
Autodesk's SDK, so not proof Maya opens a file)

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

## Until then

`tools/assimp-check.sh <reference> <ours>` remains the automated independent-reader
gate, and the Blender path (headless + the new live bridge) is the primary visual
check. Both pass today; P2-10b stays open only for the Autodesk-SDK confirmation.
