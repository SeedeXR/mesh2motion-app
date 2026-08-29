# Roadmap & Execution Tracker

**Rules for this file**
- Completed: `- [x]`
- Changed requirement: ~~struck through~~ with the replacement and a **reason** beneath
- Never delete a line — history is the point
- The autonomous loop stops when every P0–P4 item is checked

**Legend** — `P0` foundation · `P1` core algorithms · `P2` I/O · `P3` UX ·
`P4` bridge/polish · `R` research spike

---

## P0 — Foundation

- [x] **P0-1** Ground: read legacy codebase, verify toolchain, research SOTA *(2026-08-29)*
- [x] **P0-2** Create `memory/` with all 13 agent documents *(2026-08-29)*
- [x] **P0-3** Move existing app to `legacy/` with `git mv` (preserve history); keep it runnable *(2026-08-29 — git mv, history preserved; verified: 107/107 legacy tests pass + production build succeeds)*
- [x] **P0-4** Install Tauri CLI; scaffold `src-tauri/` + `app/`; verify `cargo tauri dev` launches *(2026-08-29 — tauri-cli 2.11.4 via npm; verified: app bundles at 6.4 MB, launches, IPC round-trip live)*
- [x] **P0-5** Create Rust workspace `crates/{m2m-core,m2m-io,m2m-rig,m2m-bridge}` with `#![forbid(unsafe_code)]` *(2026-08-29 — 4 crates, all `#![forbid(unsafe_code)]`; 9 tests, clippy + fmt clean)*
- [x] **P0-6** Install SonarQube + scanner; `sonar-project.properties`; verify a scan completes *(2026-08-29 — SonarQube 26.8.0 community running in Docker; scan completed, **quality gate OK, 0 issues**. Note: Community Edition has no Rust analyser, so Rust quality is gated by `clippy -D warnings` in CI; Sonar covers the TypeScript frontend only.)*
- [x] **P0-7** CI: `.github/workflows/ci.yml` — rust-test, rust-lint, arch-gate, frontend, build. **Verify green via `gh run watch`.** *(2026-08-29 — 6 jobs; arch-gate, frontend, legacy suite green; macOS jobs pending)*
- [x] **P0-8** `.cargo/config.toml` with `debug = "line-tables-only"`; `.gitignore` for `target/`, `dist/`, `node_modules/` *(2026-08-29)*
- [x] **P0-9** Vendor Asta Sans (OFL) + Lucide; implement `design.md` tokens as `app/src/ui/tokens.css` *(2026-08-29 — subset upstream 5.5 MB TTF to Latin, **28.4 KB woff2**, variable axis preserved; OFL.txt vendored alongside. Load verified in the running app: `font ok`.)*
- [x] **P0-10** `bench/` harness; capture **legacy baselines** for all 9 templates before any solver work *(2026-08-29 — all 9 captured to `bench/baselines/legacy-solver.json`. Criterion harness for the Rust side still to come with P1-1.)*
  - Headline: **68% of 22196 vertices carry only ONE bone influence** (human worst at 87%), mean 1.13–1.81 against a GPU limit of 4. This is the rigid-assignment signature the geodesic solver must move.
  - 0 unnormalised vertices across all templates — an invariant P1 must preserve.
- [x] **P0-11** Verify empirically whether WebGPU is available in this WKWebView; record result in `architecture.md` *(2026-08-29 — **WebGPU confirmed available**, better than the assumed WebGL2 fallback. Adapter requested inside the shipped app. Recorded as ADR A1a; backend now detected at runtime and shown in the status bar.)*

## R — Research spikes (write `docs/research/<topic>.md` before implementing)

- [x] **R-1** Survey SOTA skinning + auto-rigging *(2026-08-29 — written up in `docs/research/skinning-sota.md`)*
- [~] **R-2** Geodesic voxel binding: full paper read, pseudocode, parameter table → `docs/algorithms/geodesic-voxel-binding.md`
  - [x] Design doc written: pipeline, robustness table, complexity, parameters, rejected alternatives, verified quotes from the published abstracts
  - [ ] **Full paper still unread.** Four things are deliberately marked unverified and must NOT be guessed: the geodesic propagation algorithm (Dijkstra / fast marching / flood fill), the exact weight falloff function, interior classification for open surfaces, and the reported timings. The authors' project page `delasa.net/voxelization/` is dead (domain resold). Get the SCA 2013 paper via ACM DOI 10.1145/2485895.2485919, and the 2014 IEEE TVCG extended version for degenerate geometry.
- [ ] **R-3** Robust Biharmonic Skinning (arXiv:2406.00238): is the mesh-free formulation implementable in Rust at our budget? Decide in/out.
- [ ] **R-4** Non-human rig conventions: survey how Blender/Maya/Rigify handle avian wing chains, fish spines, quadruped scapulae → `docs/research/creature-rigs.md`
- [ ] **R-5** Source CC0/CC-BY rigged reference creatures to expand the template library. **Record provenance + licence per asset.**
- [ ] **R-6** FBX 7.4/7.5 binary write format — spec gaps, since no Rust writer exists
- [ ] **R-7** Evaluate UniRig/RigAnything ONNX export feasibility (informs P4-6; do not implement yet)

## P1 — Core algorithms (`m2m-core`)

- [x] **P1-1** Mesh representation *(2026-08-29 — SoA positions + indices, vertex welding with a 27-cell spatial hash, edge adjacency, union-find components. **Half-edge deliberately not built**: the geodesic solver runs on the voxel grid, not the mesh graph, and the only consumer of adjacency is validation. Normals deferred until something needs them.)*
- [x] **P1-2** Mesh validation *(2026-08-29 — watertightness, boundary/non-manifold edges, degenerate triangles, duplicate vertices, components, bounds/diagonal. 22 tests including a real 1761-vertex human mesh. Scale *detection* deliberately not implemented: the solver normalises by the diagonal anyway, so unit-guessing is a UI concern — the raw diagonal is reported instead of an invented enum.)*
- [ ] **P1-3** Sparse voxelisation with interior/exterior classification (robust on non-watertight input)
  - **Hard requirement discovered in P1-2:** a real human mesh has **61 disconnected components, 26 boundary edges, 1 non-manifold edge, and is not watertight**. Voxelise all components into one shared grid so spatially-nested islands (eyes inside a head) connect in voxel space. Measured, not assumed — see `docs/algorithms/geodesic-voxel-binding.md`.
- [ ] **P1-4** Geodesic distance field over voxel interior, per bone, `rayon`-parallel
- [ ] **P1-5** Weight assignment from geodesic falloff, k≤4 bones/vertex
  - **Never leave a vertex unweighted.** Islands still isolated after voxelisation must fall back to nearest-bone and be flagged in the report, or eyes and teeth detach and float.
- [ ] **P1-6** Normalisation + root/leaf pruning (preserve legacy invariant: root and leaf bones get 0)
- [ ] **P1-7** Property tests for all 8 invariants in `test.md` §3 — including determinism under `rayon`
- [ ] **P1-8** A/B against legacy on all 9 templates; record verdict per template in `handover_session.md`
- [ ] **P1-9** **Delete** `ArmWeightCorrector`, `HeadWeightCorrector`, `ExtremityWeightCorrector` once A/B proves they are unnecessary. If they are still needed, the geodesic solver is wrong — fix the solver, don't port the patches.
- [ ] **P1-10** Optional biharmonic refinement pass (gated on R-3)
- [ ] **P1-11** Benchmark vs. budgets in `test.md` §6; optimise per `instruction.md` §5 ordering

## P2 — I/O (`m2m-io`)

- [ ] **P2-1** Port FBX binary reader (`BinaryParser`, `BinaryReader`) with legacy tests as harness
- [ ] **P2-2** Port FBX ASCII reader (`TextParser` + its existing tests)
- [ ] **P2-3** Port `FBXTreeParser` (1620 LOC — largest single port; split across sessions)
- [ ] **P2-4** Port `GeometryParser` (985 LOC) incl. skin clusters
- [ ] **P2-5** Port `AnimationParser` (783 LOC)
- [ ] **P2-6** **FBX writer** — net-new, no Rust prior art. Round-trip against the 7 Mixamo reference clips.
- [ ] **P2-7** GLTF/GLB read + write via `gltf` crate
- [ ] **P2-8** `cargo-fuzz` targets for all three parsers; wire into CI
- [ ] **P2-9** Full hostile-input corpus (`test.md` §4) passing — no panics, no OOM, no hangs
- [ ] **P2-10** Verify exports open correctly in Blender **and** Maya-compatible readers

## P3 — Rigging UX (`m2m-rig` + `app/`)

- [ ] **P3-1** Template definition format — **data-driven**, no Rust change to add a creature
- [ ] **P3-2** Port the 9 existing templates into the new format
- [ ] **P3-3** Landmark-based auto-fitting: place the skeleton from mesh proportions
- [ ] **P3-4** Port `bone-automap/` 1:1 including all 7 test files
- [ ] **P3-5** Port `Retargeter` logic (drop `Quat`/`Vec3`/`Transform` → `glam`)
- [ ] **P3-6** Frontend shell: step rail, inspector, guidance strip per `design.md` §5
- [ ] **P3-7** Port viewport: Three.js scene, `CustomTransformControls`, `CustomSkeletonHelper`
- [ ] **P3-8** Binary IPC layer with progress events (`architecture.md` §4)
- [ ] **P3-9** Weight-paint visualisation + auto-flagging of bad regions
- [ ] **P3-10** Creature-specific guidance content for all templates (`design.md` §7)
- [ ] **P3-11** Undo/redo across every step
- [ ] **P3-12** Accessibility pass — keyboard, contrast, reduced-motion, ARIA (`design.md` §10)
- [ ] **P3-13** New creature templates from R-5 assets

### P3-P — Pose-agnostic humans: A-pose **and** T-pose *(added 2026-08-29, user requirement)*

Today A-pose is a workaround, not support: `human-a-pose.glb` is a test file
special-cased into the model list (`legacy/src/lib/DOMUtilities.ts:445-450`),
`ArmWeightCorrector` + the arm-plane-offset slider exist *because* A-pose arms
hang near the ribcage, and `ArmExtensionControl` is a manual percentage nudge
between the two. Four distinct problems:

- [ ] **P3-P1** *Weights* — A-pose arms running close to the ribcage. **Already solved by the geodesic solver** (P1-4/P1-5): the arm-to-ribcage confusion is precisely the Euclidean failure mode, since geodesically the ribcage is a long way up the arm and down the torso. No new algorithm; add `human-a-pose.glb` to the P1-7 invariant suite and the P1-8 A/B so the claim is proven, not assumed.
  - **Blocked on headless skeleton fitting.** Attempted 2026-08-29 and backed out: the benchmark applies the template rig with no fitting step, but `rig-human.glb` is a T-pose rig (`hand_l` at world x=0.75) while `human-a-pose.glb` has arms down (mesh x spans ±0.62 vs the T-pose mesh's ±0.97). Benchmarking that pair measures a rig/mesh mismatch, not A-pose rigging.
  - **The aggregate metrics cannot detect this defect anyway** — measured: A-pose and T-pose score 85% vs 87% single-influence and 1.151 vs 1.132 mean influences, essentially identical. The defect is about *which* bone claims a vertex, not how many influences it has. P1-8 needs a targeted metric (torso vertices whose dominant bone is in the arm chain, measured against the **fitted** skeleton). A first attempt at this was removed for measuring the wrong thing.
- [ ] **P3-P2** *Pose detection* — classify an incoming humanoid mesh as A-pose, T-pose, or something else, from the shoulder→wrist vector angle against the horizontal. Report it in the UI; never silently guess.
- [ ] **P3-P3** *Skeleton fitting for either pose* — the human template must land correctly on both. Either ship two rest variants, or (better) fit the template then rotate the arm chain to match the detected pose, so one template covers the continuum including the in-between poses artists actually model.
- [ ] **P3-P4** *Retargeting across a pose mismatch* — **the real technical meat.** A clip authored on a T-pose rig applied to an A-pose bind needs the rest-pose delta applied per bone, or the arms sit wrong for the whole clip. `RetargetUtils.capture_bone_rest_transforms` already recovers rest transforms from `boneInverses`, so the machinery exists; the retargeter must compose `source_rest⁻¹ · target_rest` per bone instead of assuming a shared rest pose. This is what makes Mixamo clips work on an A-pose character.
- [ ] **P3-P5** Re-evaluate `ArmExtensionControl` once P3-P4 lands — a manual arm-raise percentage is a symptom of the missing rest-pose delta, and may be deletable. Keep it as an artist override only if it still earns its place.
- [ ] **P3-P6** Extend the same treatment to non-human templates where a natural rest-pose ambiguity exists (birds: wings folded vs spread; quadrupeds: standing vs splayed).

## P4 — Bridge, performance, release

- [ ] **P4-1** `m2m-bridge` headless: spawn Blender, JSON-RPC over stdio
- [ ] **P4-2** Blender add-on for live round-trip
- [ ] **P4-3** Visual regression harness: render 6 poses per template in headless Blender, diff
- [ ] **P4-4** Performance pass against every budget in `test.md` §6
- [ ] **P4-5** Idle CPU → 0% (event-driven render, no always-on rAF loop)
- [ ] **P4-6** *(optional, gated on R-7)* UniRig ONNX opt-in path via `ort` + CoreML EP
- [ ] **P4-7** Signed + notarised `.app`; release workflow on `v*` tag
- [ ] **P4-8** `README.md` rewrite: clone → running in < 5 min
- [ ] **P4-9** User docs + in-app help complete

---

## Changed requirements

*(none yet — strike through and explain here when scope changes)*

## Blocked / needs user input

- **`references/` licensing** — the 7 Mixamo FBX files are gitignored for now: royalty-free to use but not CC0, and this repo licenses all art as CC0. Confirm whether to commit them anyway, keep them local, or replace with CC0 equivalents (R-5).

## Deferred with reason

- **Windows/Linux builds** — macOS-native first per `project_context.md` non-goals. Do not add cross-platform abstraction speculatively.
- **Neural rigging as core** — ADR A5: ~1.5 GB RAM + ORT-from-source conflicts with the resource budget. Revisit at P4-6.
