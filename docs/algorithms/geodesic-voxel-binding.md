# Geodesic Voxel Binding

**Status:** R-2 · design doc, not yet implemented (P1-3 … P1-6)
**Decision:** default skinning solver — see `memory/architecture.md` ADR A2

## Problem

Given a rest-pose mesh and a fitted skeleton, decide how strongly each bone
deforms each vertex. In artist terms: when the elbow bends, which parts of the
sleeve follow it, and how smoothly does the influence fade into the forearm and
the upper arm?

The legacy solver answers this with the **Euclidean** distance from a vertex to
each bone's midpoint, then takes the single nearest bone
(`legacy/src/lib/solvers/WeightCalculator.ts:71-80`). Euclidean distance travels
through empty space, so a hand resting beside a hip is "near" the hip bone even
though no flesh connects them. Measured on the shipping templates
(`bench/baselines/legacy-solver.json`): **68% of vertices end up with a single
bone influence**, mean 1.13–1.81 influences per vertex against a GPU limit of 4.

## Approach

Measure distance **through the inside of the mesh** instead of through space.
Two points on either side of an air gap are then correctly far apart.

```mermaid
flowchart TD
    A[rest mesh + fitted skeleton] --> B["1· sparse voxelisation<br/>rasterise triangles into a grid"]
    B --> C["2· classify voxels<br/>exterior / boundary / interior"]
    C --> D["3· rasterise bones<br/>each bone marks the voxels it passes through"]
    D --> E["4· geodesic distance per bone<br/>propagate through non-exterior voxels only"]
    E --> F["5· weights from distance falloff<br/>keep k nearest bones, k ≤ 4"]
    F --> G["6· normalise to sum 1.0"]
    G --> H["7· prune root and leaf bones to 0"]
    H --> I([skin indices u16×4, weights f32×4])
```

The critical step is **4**: propagation is confined to non-exterior voxels, so a
path from the hip to the hand must travel up the torso, along the arm and down
the forearm. The air gap is not a shortcut. This is what removes the failure
mode that `ArmWeightCorrector` and `ExtremityWeightCorrector` were written to
patch.

## Why voxels rather than a tetrahedral mesh

Bounded Biharmonic Weights needs the volume bounded by the surface to be
tetrahedralised. Real artist meshes are frequently non-watertight, non-manifold,
self-intersecting, or several disconnected components, and tetrahedralisation
either fails outright or produces garbage on exactly those inputs.

Voxelisation degrades gracefully instead: a hole in the surface makes some
voxels ambiguous, not the whole solve impossible. The paper is explicit that
this is the production motivation, and it is why Maya ships the method.

## Citation

Olivier Dionne and Martin de Lasa. *Geodesic Voxel Binding for Production
Character Meshes.* Proc. 12th ACM SIGGRAPH/Eurographics Symposium on Computer
Animation (SCA), July 2013. DOI [10.1145/2485895.2485919](https://dl.acm.org/doi/10.1145/2485895.2485919)

Extended version, focused on degenerate geometry: Dionne and de Lasa,
*Geodesic Binding for Degenerate Character Geometry Using Sparse Voxelization*,
IEEE TVCG, October 2014. [IEEE Xplore 6809992](https://ieeexplore.ieee.org/document/6809992/)

Verified from the published abstracts (2026-08-29):

> "…production meshes that may contain non-manifold geometry, be non-watertight,
> have intersecting triangles, or comprise multiple connected components."

> "…binding weights, based on the geodesic distance between each voxel lying on
> a skeleton bone and all non-exterior voxels."

> "…smooth weights at interactive rates, without time-constants, iteration
> parameters, or costly optimization at bind or pose time."

> "By decoupling weight assignment from distance computation the method makes it
> possible to modify weights interactively, at pose time, without additional
> pre-processing or computation."

That last property matters for us beyond performance: it is what lets an artist
adjust falloff and see the result immediately, which is the calibration knob
`memory/philosophy.md` insists on leaving in.

**Source note:** the authors' project page (`delasa.net/voxelization/`) is dead —
the domain now redirects to an unrelated site. Use the ACM and IEEE versions.

## Not yet verified — resolve before implementing

The abstracts do not pin these down, and **they must not be guessed**:

| Unknown | Why it matters | How to resolve |
|---|---|---|
| Geodesic propagation algorithm — Dijkstra, fast marching, or weighted flood fill | determines accuracy and whether it parallelises per bone | read the full SCA 2013 paper |
| Exact weight falloff function from distance | directly sets deformation smoothness | full paper; otherwise pick and document our own |
| Interior/exterior classification for open surfaces | the whole robustness claim rests on this | the 2014 TVCG version targets precisely this |
| Reported timings and mesh sizes | to sanity-check our budget | full paper |

Until the full text is read, this document describes the **shape** of the
algorithm, not its exact numerics. Do not cite performance figures for this
method anywhere until they come from the paper.

## Complexity

With `n` vertices, `b` bones, `v` occupied voxels:

- Voxelisation: `O(triangles)` to rasterise, `O(v)` memory for a sparse grid
- Classification: `O(v)` flood fill from the grid boundary
- Geodesic distance: `O(b · v log v)` for Dijkstra-style propagation, and the
  per-bone loop is embarrassingly parallel → `rayon`
- Weight lookup: `O(n · k)` with `k ≤ 4`

Voxel resolution is the single quality/memory dial. Memory grows roughly with
the square of resolution for a surface-sparse grid, which is how the solve is
kept inside the 1.5 GB budget in `memory/test.md` §6.

## Robustness — must be stated, not assumed

| Input defect | Expected behaviour | Test |
|---|---|---|
| non-watertight | interior classification degrades locally, solve completes | P1-7 |
| self-intersecting | unaffected; voxels are occupancy, not topology | P1-7 |
| disconnected components | each component solves; bones in no component get 0 | P1-7 |
| zero-area / degenerate triangles | skipped at rasterisation | P1-2 |
| thin features (fingers, fins, primaries) | **risk: missed below the voxel size** — needs adaptive resolution | P1-3 |
| mesh in cm vs m | scale-invariant after normalising grid to the bounding box | P1-7 invariant 7 |

The thin-feature case is the known weakness and the one to measure early: bird
primaries and fish fins are exactly the creatures this project exists to serve.

## Parameters

| Name | Range | Default | Visible effect |
|---|---|---|---|
| voxel resolution | 64–512 per longest axis | 256 | detail captured vs memory and time |
| falloff exponent | 1.0–4.0 | TBD from paper | how sharply influence fades from a bone |
| max influences | 1–4 | 4 | smoothness vs GPU cost |
| bone radius scale | 0.5–2.0 | 1.0 | how much volume a bone claims |

## Measured performance

None yet. To be filled by P1-11 with machine, input, and run count, per
`memory/docs.md` §4. The comparison target is
`bench/baselines/legacy-solver.json`.

## Rejected alternatives

| Alternative | Why it lost |
|---|---|
| Bounded Biharmonic Weights (Jacobson 2011) | needs tetrahedralisation, which fails on the exact meshes we must support |
| Robust Biharmonic / geometric fields (Dodik 2025) | best quality and mesh-free, but needs a hardware ray-tracing implementation — deferred to R-3 as an optional high-quality mode |
| Neural prediction (UniRig, RigAnything) | solves skeleton *placement*, not weights; ~1.5 GB RSS conflicts with the budget — deferred to P4-6 |
| Keep nearest-bone plus more correctors | each new creature needs new correction code; that is the defect being removed |

See `docs/research/skinning-sota.md` for the full survey.
