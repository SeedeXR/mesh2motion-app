# Mesh2Motion — User Guide

Rig and animate any creature, locally, in about a minute. This guide walks the
six steps the app takes you through, top to bottom. You never need an account
and nothing leaves your machine.

## Before you start

You need a **mesh to rig** — a `.glb`/`.gltf` or `.fbx` file of a single
creature in a roughly neutral pose (standing, limbs apart). The cleaner the
pose, the better the automatic fit. A closed fist or crossed arms will still
rig, but the weights near the overlap are harder to get right.

The app works in **metres**. FBX files are usually authored in centimetres; the
importer scales them for you, so a human should come in around 1.6–1.9 m tall.

---

## The six steps

The panel on the left is a step rail. Each step unlocks the next; you can go
back and change an earlier choice at any time (a change downstream is
recomputed, not silently kept stale).

### 1 · Import

Drop in your model, or use the file picker. Both `.glb` and `.fbx` are read.

**Your existing bones are kept.** If the file already carries a skeleton (a
Mixamo export, say), the app reports it and leaves it in place — it does not
strip it. Re-rigging is something you ask for in the next step, never something
that happens to a file behind your back.

### 2 · Choose a skeleton

Pick the template that matches your creature. Nine ship today:

| Template | For |
|---|---|
| Human | bipeds, humanoids |
| Fox / Horse | four-legged mammals |
| Bird | wings + tail |
| Spider | many-legged |
| Snake | long spines, no limbs |
| Shark | fins, aquatic |
| Kaiju / Dragon | large bipeds and winged quadrupeds |

Each template is a bone hierarchy shaped for that body plan. If none is a exact
match, pick the closest — the next step reshapes it to your mesh.

### 3 · Fit the skeleton

The template snaps to your model's proportions automatically: the spine finds
the body, the limbs find the limbs. The fitted skeleton draws over your mesh in
orange.

**Adjust by hand if you want.** Every joint has a draggable handle. Click one to
attach a move gizmo, then drag it onto where the bone should sit. The skeleton
redraws as you go, and the change carries into binding — you are moving the real
bone, not a preview. Orbit the camera freely; it holds still while you drag a
handle.

Most models need no adjustment. Reach for the handles when a joint sits outside
the mesh, or when a tapering tail or a folded wing confuses the automatic fit.

### 4 · Bind the weights

Binding decides how much each bone pulls on each vertex. Mesh2Motion uses
**geodesic voxel binding** — it measures distance *through the mesh*, not
straight through the air, so an arm held close to the ribcage does not bleed its
weight onto the chest the way simpler methods do.

Turn on the **weight-paint overlay** to check the result: each bone gets a
colour, and every vertex is tinted by the bones that move it. Look for clean
bands at the joints and no stray colour crossing a gap it should not.

Every vertex comes out with a full unit of weight — nothing is left unattached
and free to fly off when the rig moves.

### 5 · Animate

Pick a clip from the library and preview it live on your newly-rigged mesh. The
clip is **retargeted** onto your skeleton — a walk authored on a different rig
plays on yours, its motion mapped bone to bone. Scrub between clips to compare.

Playback is the only time the viewport runs a continuous frame loop; the moment
you stop, it goes back to costing nothing.

### 6 · Export

Write the finished rig to `.glb` (glTF binary) or `.fbx`. The file carries
everything: the mesh, the fitted skeleton, the skin weights, and the clip you
previewed — ready to drop into Blender, a game engine, or another DCC tool.

---

## Checking your rig in Blender

If you have Blender installed, Mesh2Motion can import an export into a headless
Blender and report what it found (bone count, mesh vertices, total weight) — an
independent second opinion that the file is correct. This is optional; the core
flow never needs Blender.

## Tips and gotchas

- **A limb rigs badly** → go back to Fit and drag the joints onto the limb; a
  skeleton that starts inside the mesh binds far better.
- **Weights bleed between nearby parts** → check the pose. Two body parts
  touching in the import (a hand resting on a hip) share vertices the solver
  cannot tell apart. Separate them in your source model and re-import.
- **The model is enormous or tiny** → it was probably authored in the wrong
  unit. The importer assumes FBX is centimetres and glTF is metres, per each
  format's convention.
- **An imported animation looks wrong on an A-pose character** → clips authored
  on a T-pose rig can sit slightly off on an A-pose bind. Pose-aware retargeting
  is on the roadmap.

## Getting help

The guidance strip under each step says what to do and why, in place. Steps 3
(Fit) and 4 (Bind) are where most rigs are won or lost, so they carry the most
guidance.
