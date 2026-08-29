# R-3: Robust Biharmonic Skinning — in or out?

**Decision: OUT.** Not implementable at this project's platform or budget.
**Date:** 2026-08-30 · **Closes:** R-3, and P1-10 with it.

## The question

`docs/research/skinning-sota.md` identified *Robust Biharmonic Skinning Using
Geometric Fields* as the most attractive result in the survey: bounded-biharmonic
quality, no tetrahedralisation, and explicitly robust on open surfaces, triangle
soups and point clouds. P1-10 reserved it as an optional "high quality" mode
pending this decision.

## Source

Ana Dodik, Vincent Sitzmann, Justin Solomon, Oded Stein (MIT CSAIL / USC).
*Robust Biharmonic Skinning Using Geometric Fields.* ACM TOG,
DOI [10.1145/3771928](https://doi.org/10.1145/3771928).
arXiv [2406.00238](https://arxiv.org/abs/2406.00238), v3 dated 3 Aug 2026.

Read from the full text this time, not the abstract — the earlier survey entry
was written from the abstract and left the implementation requirements marked
unverified. This resolves them.

## What it actually requires

From the paper's own Appendix A, *Implementation Details*:

| Component | Dependency |
|---|---|
| Main algorithm | **PyTorch** |
| Radius + closest-point queries | **custom CUDA kernels** |
| Hardware ray tracing | **OptiX** (Parker et al. 2010) and **OWL** (Wald 2020) |
| Closest-point BVH | **NVIDIA Warp** |
| Generalized winding numbers | libigl |
| Python/CUDA interop | nanobind |

Evaluated on "an Intel i9-13900 CPU, 32 GB of memory, and an Nvidia GeForce
RTX 4090".

The ray tracing is not incidental. The paper lists as a core contribution "the
use of hardware-accelerated ray tracing for a geometry-aware function
parameterization", and the kernel carries a visibility term
`k_rt(x, xi) = V(x ↔ xi) · exp(-‖x - xi‖² / 2σ²)` — a ray is traced between
every candidate point pair to test mutual visibility.

Being precise about what the paper attributes to what: the **robustness** claim
is attributed to being mesh-free, i.e. avoiding tetrahedralisation. The
visibility term is what makes the parameterisation *geometry-aware* — it stops
weights bleeding between parts that are close in space but not connected
through the volume. Both matter, and the ray tracing is load-bearing for the
second, but "the visibility test is the source of the robustness" would be an
inference rather than something the paper says.

## Reported performance

| Mesh | This method | QHW | BBW |
|---|---|---|---|
| Bunny | **71.74 s** | 7.25 min | 18.32 min |
| Gear | **32.2 s** | — | FastTetWild alone, at boundary-respecting settings: 1.78 h |

A genuine 6–15× speedup over the finite-element methods, and it succeeds on
meshes where they fail outright. But the absolute figures are tens of seconds.

Caveat on that table, from the paper: matching internal vertex counts against
tetgen required a binary search on its quality parameter, and they "were unable
to match the internal mesh density in three cases". So the comparison is not
uniformly like-for-like.

## Why it is out

**1. The GPU stack is NVIDIA-only, and this project is macOS/Apple Silicon.**
CUDA, OptiX and OWL have no macOS implementation. (NVIDIA Warp does run on
macOS in CPU-only mode, so the blanket claim would be wrong — it is specifically
the CUDA/OptiX/OWL path that has no story here.) Metal has its own ray-tracing
API, so the visibility kernel could in principle be rewritten — but that means
reimplementing the paper's central contribution on a different hardware API,
not adopting it. That is a research project, not a feature. **This is the
decisive reason.**

**2. It is roughly 6× over the applicable time budget.** `memory/test.md` §6 has
two bind-skin rows, and the right one to compare against is the **high-quality**
row — ≤ 12 s, 2.5 GB, whose note literally reads "biharmonic refinement" — not
the 3 s fast path, because P1-10 reserved this method as exactly that optional
mode. Against 12 s the paper's 71.74 s Bunny is ~6× and the 32.2 s Gear ~2.7×.

That is a real gap but a smaller one than a first pass suggested, and it is the
condition most likely to fall away: a Rust implementation without PyTorch
overhead could plausibly close 6×. **Platform, not speed, is what decides this.**

**3. PyTorch fails the same test as UniRig.** ADR A5 already deferred neural
rigging because a multi-hundred-megabyte inference stack conflicts with the
resource budget. This carries the same weight for the same reason.

**4. The robustness we needed, we already have.** The paper's motivating failure
is tetrahedralisation on non-watertight and self-intersecting input — Blender's
"Bone Heat Weighting: Failed to find solution" is quoted directly. Geodesic
voxel binding was chosen (ADR A2) precisely because voxelisation sidesteps that
class of failure, and P1-3 measured it: the reference character is **not**
watertight (26 boundary edges, one non-manifold edge, 61 components) and solves
without incident. We are not paying a robustness cost that this method would buy
back.

## What would change the decision

- A CPU or Metal implementation of the visibility kernel that lands inside the
  time budget. The paper notes the method "inherits the practically sub-linear
  scaling of ray-tracing with respect to boundary resolution", so the ceiling
  may not be as far away as the reference timings suggest.
- Artists reporting that geodesic weights are not smooth enough. P1-8 measured
  2.24–3.15 mean influences across the nine templates against the legacy
  solver's 1.13–1.81; if that proves insufficient in practice, quality becomes
  worth paying for.
- **Reference code, which the authors have committed to releasing.** The paper
  states "We will release the code upon acceptance", and it has since been
  accepted (the TOG DOI above). A released reference implementation would remove
  most of the risk in judging portability, and is the single thing most likely
  to reopen this. Worth checking back for.

Until one of those, a second solver is speculative — and a second solver is not
free: two weighting paths to test, benchmark and keep correct.

## One thing worth stealing regardless

The paper incorporates **artist weight painting as Dirichlet boundary conditions
inside the optimisation**, rather than as a post-process. That idea is
independent of the ray-tracing machinery and fits the calibration-knob principle
in `memory/philosophy.md`. Recorded for the P3 weight-painting work.
