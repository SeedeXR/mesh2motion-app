# Project Context

## Vision

A native macOS desktop rigging application that does for **every creature** what
Mixamo did for humans — and does it locally, fast, and without an account.

Mixamo rigs bipedal humanoids and stops there. Blender's Rigify is powerful and
brutally unintuitive. The gap is a tool where an artist drops in a bird, a shark,
a horse, or a dragon and gets a production-usable rig with clean weights in under a
minute, with clear guidance at every step.

## What exists today (verified 2026-08-29)

`legacy/` — a working TypeScript + Three.js web app, 30,780 LOC across 148 `.ts`
files, deployed at app.mesh2motion.org. It already does more than Mixamo on
creature coverage:

- **9 template skeletons**: human, fox, bird, horse, shark(fish), dragon, kaiju, snake, spider
- 18 model variations (`legacy/static/models-variation/`) — e.g. `human-female`, `fox-horse`, `bird-eagle`, `kaiju-t-rex`
- A hand-written FBX parser (~4,100 LOC, `legacy/src/lib/io/fbx/`)
- Animation retargeting with bone auto-mapping (Mixamo / Rigify / Mesh2Motion vocabularies)
- GLB export

**Its ceiling is the skinning solver.** `WeightCalculator.ts:71-80` assigns each
vertex to exactly one nearest bone by midpoint distance, then three hand-written
correctors (`ArmWeightCorrector`, `HeadWeightCorrector`, `ExtremityWeightCorrector`)
patch the specific artefacts that approach produces, and a smoother blends seams.
Every new creature type needs new correctors. That does not scale.

## Objectives

| # | Objective | Success metric |
|---|---|---|
| O1 | Native desktop app, Tauri + Rust | ships as a signed `.app`, ≤ 40 MB, idle RSS ≤ 250 MB |
| O2 | Research-grade skinning replacing closest-bone | geodesic voxel binding; visibly better weights on all 9 templates vs. `legacy/` A/B |
| O3 | Intuitive rigging for non-humans | a new user rigs a bird correctly in < 3 min without docs |
| O4 | FBX import **and export** | round-trips the 7 Mixamo reference clips + Maya/Blender read the export |
| O5 | Blender DCC bridge | live send/fetch rig + animation to a running Blender |
| O6 | Performance | 50k-vertex rig ≤ 3 s, ≤ 1.5 GB peak, 0% idle CPU |
| O7 | Dark-themed, guided UX | Lucide icons, 42dot/Asta Sans, per-step instruction panel |
| O8 | **Pose-agnostic humans** | the same model rigs correctly whether authored in A-pose or T-pose, and T-pose-authored clips retarget onto an A-pose bind without arm drift |
| O9 | **An existing rig is preserved, never silently discarded** *(user requirement, 2026-08-30)* | importing an already-rigged model keeps its skeleton, bone names, hierarchy and skin weights by default; re-rigging is a deliberate choice, not the default, and nothing is lost without the user asking |

### O9 — why this is a change, not a port

The legacy has both behaviours but makes the user pick the right one, and
strips the rig if they pick wrong:

- `ModelCleanupUtility.strip_out_all_unecessary_model_data` converts every
  `SkinnedMesh` to a plain `Mesh` and deletes the `skinIndex` and `skinWeight`
  attributes — the existing rig is gone.
- `ModelCleanupUtility.strip_out_retargeting_model_data` preserves the
  `SkinnedMesh` and its bone hierarchy, but only the retarget flow uses it.
- `ModelAnalysisReport` warns: *"Mesh is already rigged. This workflow drops the
  existing skeleton - use \"Use Your Rigged Model\" to keep it."*

So today preservation is opt-in and losing the rig is the default for anyone who
does not read the warning. **The requirement inverts that.** The read path
already has everything needed — `model::parse_all` gives the bone hierarchy and
`skin::parse_all` gives clusters, weights and bind matrices — so this is a
workflow decision, not missing capability.

## Deliverables

1. Rust workspace (`crates/`) — `m2m-core` (solver/math), `m2m-io` (FBX/GLTF), `m2m-rig` (templates/retarget), `m2m-bridge` (Blender)
2. Tauri shell (`src-tauri/`) + TypeScript/Three.js frontend (`app/`)
3. Expanded creature template library with guided fitting
4. Blender bridge add-on
5. Full test suite per `test.md`, CI on GitHub Actions, SonarQube gate
6. Documentation per `docs.md`

## Target users

1. **Indie game devs** — need rigged, animated creatures fast; not riggers.
2. **3D artists / hobbyists** — can model, find rigging opaque.
3. **Technical animators** — want control and clean weights they can hand-tune.
4. **Studios** — batch rigging of background creatures.

## Constraints (verified, not assumed)

- Dev machine: **Apple M4, 10 cores, 16 GB RAM, macOS 26.6.2, ~34 GB free disk**
- Rust 1.96.0, Node 22.16.0, Xcode 26.5 (Metal toolchain present)
- Blender at `/Applications/Blender.app` (headless via `Contents/MacOS/Blender -b`)
- Tauri CLI and SonarQube **not yet installed** — install is a todo item
- `target/` must be watched; see `session_start.md` §5
- Licences: code MIT, art assets CC0. **Any new asset must be CC0/CC-BY or OFL, with provenance recorded.**
- 42dot Sans was renamed Asta Sans (Feb 2026) and pulled from Google Fonts — vendor the OFL files, no CDN

## Evaluation criteria

A change is good if it: makes rigging a non-human creature easier, makes weights
better on a real messy mesh, makes the app faster or lighter, or makes the codebase
smaller. A change that does none of these does not belong.

## Explicit non-goals (for now)

- Windows/Linux builds (macOS-native first; do not add cross-platform abstraction speculatively)
- Cloud/account features
- A full animation authoring timeline (retarget and export, not keyframe authoring)
- Neural skeleton prediction as a *core* dependency — deferred to an opt-in phase (see `todo.md` P3)
