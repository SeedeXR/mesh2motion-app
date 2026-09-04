# Research: marker-based skeleton solving (Mixamo & friends)

**Date:** 2026-09-05 · **Status:** research + concrete plan · **Informs:** the
marker-placement auto-rig flow (`m2m_rig::fit::fit_from_markers`)

Complements the existing research: skinning is settled (geodesic voxel binding,
see `docs/algorithms/geodesic-voxel-binding.md`, `docs/research/skinning-sota.md`)
and neural rigging is deferred (`docs/research/onnx-feasibility.md`). This covers
the dimension those don't: **how the skeleton is fit to the mesh from sparse
user markers**, and why our marker solve fails at the feet/ankles.

## The reported problem

Marker placement "fails to solve for feet and ankles." Root cause, confirmed in
code: `fit_from_markers` is **mesh-blind**. It takes only the rest pose, the
parent tree, and the markers. Its own doc says it plainly — "a bone past the last
marker on its chain rigid-follows it ... a foot by its knee." The human marker set
is chin / wrists / elbows / knees / groin, so the **foot, ankle and toes are
unmarked**: they keep their scaled-rest offset below the knee marker and are never
placed onto the mesh's feet nor grounded. A model whose shin proportion differs
from the template floats or sinks its feet.

Our **other** path, the automatic `fit_template`, already does the right thing
here: it voxelises the mesh and runs `fit_limbs` + `ground_bone` + `ankle_height`
to swing each leg onto the mesh's actual limb and stand the toe on the ground. The
marker path simply doesn't use any of it.

## How Mixamo actually does it

Mixamo's auto-rigger is **markers + mesh**, not markers alone:

- **Sparse markers constrain the key joints.** The standard set is chin, wrists,
  elbows, knees, groin (higher LODs add head-top, neck, shoulders, ankles). Precise
  placement guidance: chin at jaw centre, wrist at the wrist joint, elbow at the
  bend centre, knee at the kneecap, groin at the hip joint. Symmetry mirrors L/R.
- **The mesh fits everything the markers don't pin.** Feet, ankles, spine
  subdivisions, fingers and ground contact come from the geometry, not from the
  handful of markers. This is why Mixamo needs no foot marker yet stands the feet
  on the ground: it fits the leg to the mesh and grounds it.
- **Skinning** is heat-diffusion ("bone glow") over a volumetric mesh, ~1 minute,
  "80–100%" of final quality. (We already do the equivalent with GVB.)
- **Scope:** Adobe explicitly excludes quadrupeds, wings, tails, extra limbs. Our
  template model is our advantage over Mixamo here.

Sources at the end.

## The deterministic classic: Pinocchio (Baran & Popović, 2007)

The seminal auto-rig, and the closest match to what we can implement in Rust with
no training data:

1. Approximate the **medial surface** with a signed-distance field; pack spheres
   centred on it inside the character; connect sphere centres into a graph.
2. **Embed** a generic skeleton into that graph by minimising a discrete penalty
   (≈9 hand-built terms: bone-length ratios, symmetry, keeping joints inside the
   mesh, orientation), then refine with continuous optimisation.
3. Skinning by **heat diffusion** (bone as heat source, steady-state over the
   volume).

The takeaway for us: the winning idea is *fit the skeleton to the volume of the
mesh*, with the template supplying structure and per-joint penalties supplying the
"where should this joint sit" rules. Our `fit_template` is a lighter version of
exactly this (voxel grid instead of sphere-packing, per-chain rules instead of a
global penalty). Markers are just **hard constraints** layered on top: pin the
marked joints, embed the rest.

## The modern SOTA (context, not a recommendation)

- **RigNet (2020)** — a GNN predicts joints, topology and skin weights from the
  mesh, no template. High quality, needs training + inference (already assessed as
  deferred in `onnx-feasibility.md`).
- **UniRig / RigAnything / Skin Tokens (2025–26)** — autoregressive, template-free
  rigging for arbitrary assets; the frontier, but heavy and ML-bound.
- These reinforce the direction, not the implementation: they all learn "put the
  skeleton in the volume." We get most of that deterministically from
  template + markers + voxel embedding, which is the right call for a local,
  no-training, arbitrary-creature app.

## Recommendation: one marker-constrained, mesh-fitted pipeline

Make `fit_from_markers` do what Mixamo does — markers pin, mesh fits the rest —
by giving it the mesh and reusing the machinery `fit_template` already has.

**Stage A — markers pin the key joints** (today's solve): uniform scale+translation
from the marked correspondences, then per-chain delta propagation so every marked
joint lands exactly and bracketed joints blend. Unchanged.

**Stage B — the mesh fits the unmarked joints** (new): voxelise the mesh once, then
for each chain refine the run **below the last marker** against the geometry, with
the marked joint held fixed as the anchor:

- **Legs:** from the marked knee down, swing/extend the shin+foot to the mesh's
  actual foot and **ground the toe** — reuse `ground_bone` (the lowest chain bone)
  + `ankle_height` (posture-aware ankle ride) + the mesh's lowest surface under the
  leg. This is the direct fix for "feet/ankles don't solve."
- **Spine / neck / head:** run `refine_spine` + `snap_spine_into_mesh` between the
  groin and chin markers so the back sits inside the torso.
- **Unmarked whole chains** (fingers, tails, ears): keep rigid-follow — cheap and
  good enough; the mesh refine is only worth it for the load-bearing distal joints.

Marked joints are never moved by Stage B, so the user's placement is always
honoured; the mesh only decides what the user didn't.

**Humans vs animals.** The mechanism is creature-agnostic because it is
template-driven (chains carry `posture`, `role`, `side`; `ground_bone`/`ankle_height`
already handle plantigrade/digitigrade/unguligrade). Two tiers:

- **Human & bipeds:** the Mixamo marker set + Stage B (this plan).
- **Quadrupeds / exotic (fox, horse, bird, spider, dragon…):** Mixamo can't rig
  these at all; we can. Default them to the **automatic** `fit_template` (already
  mesh-fitted, already grounds feet), and *optionally* add a per-creature marker
  set later (e.g. paws, hips, shoulders, tail tip) that feeds the same Stage A→B.
  Same pipeline, different marker table — no new algorithm.

**Implementation sketch**

- `fit_from_markers(template, rest, parents, markers, mesh, resolution)` — add the
  mesh; build the voxel grid once; after Stage A, call a new
  `refine_unmarked_against_mesh(fitted, mesh, grid, template, marked_set)` that runs
  the limb/foot + spine refinement only on joints not in `marked_set` and below the
  last marker of their chain.
- Pipeline `fit_from_markers` gains the model bytes (it already loads the rig; add
  `mesh_of(import::load(model))`). Command + IPC pass the model path (the frontend
  already holds `loaded.path`).
- Tests: a marked human whose shin is longer/shorter than the template still stands
  its toe on the mesh ground; marked joints stay exactly on their markers.

**Cost:** one voxelisation (already ~the auto-fit cost, 128³) added to the marker
solve. Acceptable — the marker solve is otherwise near-instant.

## Sources

- Mixamo auto-rigger (markers, process): [Adobe help](https://helpx.adobe.com/creative-cloud/help/mixamo-rigging-animation.html),
  [VRChat quick-start](https://vrchat.fandom.com/wiki/Quick_Start_-_Mixamo_Avatar_Creation),
  [MCV interview](https://mcvuk.com/development-news/interview-mixamo/)
- Pinocchio: [Baran & Popović 2007 (PDF)](https://www.cs.toronto.edu/~jacobson/seminar/baran-and-popovic-2007.pdf),
  [SIGGRAPH history](https://history.siggraph.org/learning/automatic-rigging-and-animation-of-3d-characters-by-baran-and-popovic/)
- RigNet: [project](https://zhan-xu.github.io/rig-net/), [code](https://github.com/zhan-xu/RigNet)
- Template-free frontier: [RigAnything](https://arxiv.org/pdf/2502.09615), [UniRig](https://arxiv.org/pdf/2504.12451)
- Skinning weights: [Geodesic Voxel Binding](https://www.researchgate.net/publication/262271901_Geodesic_voxel_binding_for_production_character_meshes),
  heat diffusion (Pinocchio, above)
- Quadruped rigging context: [Mixamo excludes quadrupeds (Adobe help)](https://helpx.adobe.com/creative-cloud/help/mixamo-rigging-animation.html)
