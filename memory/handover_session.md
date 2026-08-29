# Session Handover Log

Newest entries at the top. Every session appends one entry before exiting.
Timestamps are local (macOS, `date "+%Y-%m-%d %H:%M:%S"`).

---

## Session 001 — 2026-08-29

**Started:** 2026-08-29 15:52:11
**Ended:** 2026-08-29 16:26:41
**Focus:** Grounding, architecture decisions, memory bootstrap

### Completed
- **P0-1** Grounding pass over the legacy codebase and toolchain
- **P0-2** All 13 `memory/` documents created
- **R-1** SOTA survey for skinning + auto-rigging

### Observed facts (verified this session, not assumed)
- Legacy app: 30,780 LOC TS across 148 files at commit `1271226`
- **Core defect located:** `legacy/src/lib/solvers/WeightCalculator.ts:71-80` — rigid nearest-bone, one bone per vertex, Euclidean distance to `bone_midpoint_to_child`. The three weight correctors exist to patch this one weakness.
- 9 templates in `SkeletonType`; `Fish` maps to `rig-shark.glb`
- FBX parser is ~4,100 LOC hand-written, handles ASCII **and** binary
- Toolchain present: Rust 1.96.0, Node 22.16.0, Xcode 26.5 + Metal, Blender.app
- Toolchain **missing**: Tauri CLI, SonarQube (→ P0-4, P0-6)
- **No CI exists** — `.github/` contains only `FUNDING.yml` (→ P0-7)
- Machine: Apple M4, 10 cores, 16 GB RAM, macOS 26.6.2, **~34 GB free disk**

### Corrections to stated premises
- **Font:** "42dot Sans" was renamed **Asta Sans** (Feb 2026) and **removed from Google Fonts**. Still SIL OFL. Must be vendored — a CDN link would not work. Confirmed with user.
- **Rust FBX crates:** `fbxcel` is binary-only, read-only, no ASCII, no export; `fbxcel-dom` is v0.0.6. Porting the legacy parser is the lower-risk path (ADR A3).

### Decisions (user-confirmed)
- **A1** Rust compute core + Three.js viewport, not full wgpu — preserves ~3k LOC of working interaction code
- **A2** Geodesic voxel binding as the default solver
- **A5** Neural rigging (UniRig) deferred to opt-in P4-6

### Research findings
- Dionne & de Lasa, *Geodesic Voxel Binding*, SCA 2013 — Maya's method, robust on non-watertight meshes
- Dodik/Sitzmann/Solomon/Stein, *Robust Biharmonic Skinning Using Geometric Fields*, TOG 2025 (arXiv:2406.00238) — mesh-free, no tetrahedralisation
- UniRig (SIGGRAPH 2025) / RigAnything — skeleton prediction across bipeds, quadrupeds, birds, insects, marine

### Blockers
None.

### Next session starts at
**P0-3** — `git mv` the existing app into `legacy/`, keeping it runnable.
Then P0-4 (Tauri scaffold) and P0-5 (Rust workspace).

### Notes
- `legacy/` is a **test dependency**, not dead code — it is the A/B baseline for P1-8. Do not delete it.
- Capture legacy benchmarks (**P0-10**) *before* touching the solver, or the A/B comparison has no baseline.
