# Research: State of the art in automatic skinning and rigging

**Task:** R-1 · **Date:** 2026-08-29 · **Status:** complete, feeds R-2/R-3

## Question

The legacy solver assigns each vertex to a single nearest bone by Euclidean
distance (`legacy/src/lib/solvers/WeightCalculator.ts:71-80`). What should replace
it, given the constraints: runs locally on an M4 laptop, ≤ 1.5 GB peak for a
50k-vertex mesh, ≤ 3 s, must work on **arbitrary creatures**, and must survive the
non-watertight meshes artists actually produce?

## Why the current approach fails

Euclidean distance travels through empty space. A hand resting near a hip is
"close" to the hip bone even though no flesh connects them, so the hip bone claims
hand vertices. Every one of the legacy correctors patches an instance of this one
defect:

| Corrector | Artefact it patches | Underlying cause |
|---|---|---|
| `ExtremityWeightCorrector` | finger bones grabbing knuckle/palm vertices | Euclidean jump |
| `ArmWeightCorrector` | arm bones stealing ribcage vertices in A-pose | Euclidean jump |
| `HeadWeightCorrector` | head/neck boundary misplaced | single-bone assignment |
| `WeightSmoother` | hard seams between regions | single-bone assignment |

Two root causes, four patches, and a new patch for every new creature. Fixing the
causes is strictly cheaper than continuing to patch.

## Candidates evaluated

### 1. Bounded Biharmonic Weights (BBW)
Jacobson, Baran, Popović, Sorkine — SIGGRAPH 2011.

Constrained energy minimisation producing smooth, non-negative, partition-of-unity
weights. The quality reference the field measures against.

**Rejected as the default.** Requires tetrahedralising the volume bounded by the
surface. Artist meshes are frequently non-watertight, self-intersecting, or triangle
soup, and tetrahedralisation either fails or produces artefacts on exactly those
inputs. Solving the linear system is also expensive at our budget.

### 2. Geodesic Voxel Binding — **chosen as default**
Dionne & de Lasa — SCA 2013 (ACM SIGGRAPH/Eurographics Symposium on Computer
Animation). DOI 10.1145/2485895.2485919.

Voxelises the input, classifies interior voxels, and computes geodesic distance
**through the voxel interior** from each bone to each interior voxel. Weights fall
off with that geodesic distance.

**Why it wins here:**
- Geodesic distance cannot jump across empty space — this alone removes the root
  cause behind `ExtremityWeightCorrector` and `ArmWeightCorrector`.
- Sparse voxelisation degrades gracefully on non-watertight input rather than
  failing, which is why Maya ships this method for production meshes.
- Embarrassingly parallel per bone → maps directly onto `rayon`.
- Voxel resolution is a single quality/memory dial, which keeps it inside the RSS
  budget on large meshes.
- No linear solve, no tetrahedralisation, no external solver dependency.

**Known limits:** blockiness at low voxel resolution; thin features (fingers, fins,
feathers) can be missed if the grid is too coarse. Adaptive resolution and a
post-smoothing pass are the mitigations. Quality is below BBW at equal effort.

### 3. Robust Biharmonic Skinning Using Geometric Fields — **deferred to R-3**
Dodik, Sitzmann, Solomon, Stein — arXiv:2406.00238 (v. Aug 2026).

> "We introduce a mesh-free and robust automatic skinning technique that generates
> weights comparable to the current state of the art, but works reliably even on
> open surfaces, triangle soups, and point clouds where current methods fail. We
> achieve this through the use of a specialized Lagrangian representation enabled
> by the advent of hardware ray-tracing, which circumvents the need for finite
> elements while optimizing the biharmonic energy and enforcing boundary
> conditions."

This is the most attractive result found: BBW-grade quality, no tetrahedralisation,
and explicitly robust on open surfaces and triangle soup. It also supports artist
weight painting *during* optimisation, which fits the "leave the calibration knob"
principle in `philosophy.md`.

**R-3 resolved it: OUT.** The full text was read on 2026-08-30 — see
`docs/research/robust-biharmonic-decision.md`. In short: the implementation is
PyTorch plus custom CUDA plus **OptiX/OWL/Warp**, all NVIDIA-only, on a project
targeting Apple Silicon; and it reports **71.74 s** on Bunny against a 3 s
budget. The robustness it buys — surviving non-watertight and self-intersecting
input — is what voxelisation already gives us, measured in P1-3.

Worth stealing regardless: the paper folds artist weight painting into the
optimisation as Dirichlet boundary conditions rather than applying it as a
post-process. Recorded for the P3 weight-painting work.

### 4. SkinCells: Sparse Skinning using Voronoi Cells
Larionov et al. — Computer Graphics Forum 2025. Noted, not yet evaluated.

### 5. Neural skeleton prediction — deferred to P4-6
- **UniRig**, SIGGRAPH 2025 (Tsinghua + Tripo) — autoregressive skeleton generation
  with bone-point cross-attention and skeleton-tree tokenisation. Trained on Rig-XL,
  which spans **bipeds, quadrupeds, birds, insects and static objects**.
- **RigAnything**, arXiv:2502.09615 — template-free autoregressive rigging covering
  bipedal, quadrupedal, **avian, marine**, insectoid and rigid objects. Explicitly
  positioned against RigNet's failure on tails and wings.
- **Anymate** (arXiv:2505.06227), **Puppeteer**, **Skin Tokens** (arXiv:2602.04805).

These directly target the "rig any creature" goal and are the strongest long-term
answer for **skeleton prediction** (which the geodesic solver does not address — it
solves weights given a skeleton).

**Deferred, not rejected** (ADR A5): running them means ONNX Runtime with the CoreML
execution provider, and CoreML EP is not in Microsoft's prebuilt binaries, so ORT
must be built from source. Combined with several hundred MB of weights and ~1.5 GB
inference RSS, this conflicts with the stated resource budget. It belongs behind an
explicit opt-in, after the template + geodesic path is solid.

## Decision

1. **Now (P1):** geodesic voxel binding as the default solver. Removes the root
   cause, fits the budget, no exotic dependencies.
2. **Next (R-3):** evaluate robust biharmonic geometric fields as an optional high
   quality mode. Read the full PDF before committing.
3. **Later (P4-6):** neural skeleton prediction as an opt-in download.

Skeleton *placement* stays template + landmark-fitting driven (P3-3) until the
neural path is justified.

## Sources

- Dionne & de Lasa, *Geodesic Voxel Binding for Production Character Meshes*, SCA 2013 — https://dl.acm.org/doi/10.1145/2485895.2485919
- Dodik, Sitzmann, Solomon, Stein, *Robust Biharmonic Skinning Using Geometric Fields* — https://arxiv.org/abs/2406.00238
- Jacobson et al., *Bounded Biharmonic Weights for Real-Time Deformation*, SIGGRAPH 2011
- Larionov et al., *SkinCells: Sparse Skinning using Voronoi Cells*, CGF 2025 — https://onlinelibrary.wiley.com/doi/10.1111/cgf.70381
- *One Model to Rig Them All: Diverse Skeleton Rigging with UniRig*, SIGGRAPH 2025 — https://arxiv.org/pdf/2504.12451 · https://github.com/VAST-AI-Research/UniRig
- *RigAnything: Template-Free Autoregressive Rigging* — https://arxiv.org/pdf/2502.09615
- *Anymate: A Dataset and Baselines for Learning 3D Object Rigging* — https://arxiv.org/pdf/2505.06227

Accessed 2026-08-29.

## Open questions

- Voxel resolution needed to resolve thin features (bird primaries, fish fins,
  fingers) without exceeding the RSS budget → measure in P1-3.
- Does geodesic falloff alone produce acceptable shoulder/hip deformation, or is a
  smoothing pass still required? → P1-8 A/B answers this empirically.
- Full-PDF read of arXiv:2406.00238 for actual speed/quality numbers → R-3.
