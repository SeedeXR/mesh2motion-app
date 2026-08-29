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
- [x] **P1-3** Sparse voxelisation with interior/exterior classification *(2026-08-29)*
  - Conservative triangle-AABB rasterisation (Akenine-Möller 13-axis SAT), then a 6-connected exterior flood fill from the padded grid boundary; interior is whatever neither reaches.
  - **The 61-component requirement is met:** all components share one grid, so nested islands connect in voxel space. Tested directly with a small cube fully inside a large one — surfaces disconnected, interiors joined.
  - **`DEFAULT_RESOLUTION = 256`, measured not guessed.** The deciding metric is interior/surface ratio: 0.08 at res 32 (shell-dominated, nothing for the geodesic field to traverse) vs 2.69 at 256. Full sweep in `docs/algorithms/geodesic-voxel-binding.md`.
  - **Physical validation:** interior volume at 256 scales to **56 litres** for a 1.75 m human. A 60 kg person displaces ~60 L.
  - Non-watertight input (26 boundary edges) does **not** leak — conservative rasterisation seals sub-voxel holes. A face-sized hole does leak, and that limit is tested both ways.
  - **Axis-aligned geometry needed three separate fixes** before the shell sealed: the rasterisation AABB excluded the voxel containing a boundary face, the overlap test had no epsilon, and world-space rasterisation lost precision far from the origin. A plain cube had no shell at all at resolutions 13/18/20 before this. Regression sweep: 1476 cases (scale × offset × rotation × resolution).
- [x] **P1-4** Geodesic distance field over voxel interior, per bone, `rayon`-parallel *(2026-08-29)*
  - Dijkstra over a compact graph of non-exterior voxels, 26-connected with true Euclidean step lengths. Surface voxels are included — vertices sit on the surface, so excluding them would strand every vertex.
  - **190 ms** for 7399 verts × 66 bones at resolution 256; **1.9 MB** retained.
  - **Memory was the design constraint:** a per-bone field over the whole grid is 900 MB. Only non-exterior voxels participate (201k of 3.4M) and only vertex distances are retained; the field is per-thread scratch.
  - **Measured payoff: the dominant bone changes for 14.6% of vertices vs Euclidean, worst path ratio 19.4×** — on a T-pose model, the case most favourable to Euclidean.
  - `unreachable_bones()` and `unreachable_vertices()` surface the two failure modes (bone outside the mesh; island the grid never connected) instead of silently producing dead limbs.
  - **Known limit, measured and pinned:** surfaces closer than ~1.5 voxels leak into each other, restoring the Euclidean shortcut. At the default resolution on a 1.75 m human that is a ~1 cm floor — fine for an A-pose arm at 2–5 cm, not for an arm actually touching the body.
- [x] **P1-5** Weight assignment from geodesic falloff, k≤4 bones/vertex *(2026-08-30)*
  - Modified Shepard over the k nearest bones, cutoff at the surplus (k+1)-th distance so weight reaches exactly zero there and the blend does not step at the fourth influence. **The falloff function is our choice, not the paper's** — the published abstracts do not state one (R-2 open).
  - **Measured against the legacy baseline on the same model and rig: single-influence vertices 87% → 9.0%, mean influences 1.13 → 3.66.**
  - **Never leave a vertex unweighted.** Islands still isolated after voxelisation must fall back to nearest-bone and be flagged in the report, or eyes and teeth detach and float.
- [x] **P1-6** Normalisation + root/leaf pruning *(2026-08-30 — normalisation is part of the falloff pass. Pruning is a caller-supplied boolean mask rather than name inspection: `m2m-core` must not carry a naming convention. The legacy invariant is preserved by whoever builds the mask.)*
- [x] **P1-7** Property tests for all 8 invariants in `test.md` §3 *(2026-08-30)*
  - All 8 covered, on synthetic geometry, the real 7399-vertex character, **and** randomised `proptest` generators over scale/offset/rotation/resolution/falloff/bone-count.
  - **Invariants 7 and 8 needed splitting into an exact half and a converging half.** Bone *assignment* is exactly scale-invariant and exactly mirror-symmetric (0 mismatches over 480 random poses). Weight *values* only converge, because the geodesic path is a chain of discrete voxel steps. Both errors halve per resolution doubling — first-order, so discretisation rather than bias — and the tests assert **convergence**, not a fixed tolerance.
  - `proptest` earned its place immediately: it found a scale-invariance failure the fixed-fixture test could not reach and shrank it to a minimal case (base 0.05, factor 26.0).
- [x] **P1-8** A/B against legacy on all 9 templates *(2026-08-30)* — **every template improves on both metrics**, verdicts below. Pinned as a test (`crates/m2m-core/tests/template_ab.rs`) that fails per-template if the solver regresses; mutation-verified by regressing the solver to rigid assignment.

| template | verts | weightable bones | single-influence % | mean influences | raw mean | ms |
|---|---|---|---|---|---|---|
| human | 7399 | 52 | 86.8 → **9.5** | 1.13 → **2.46** | 3.66 | 289 |
| fox | 1222 | 38 | 49.1 → **3.7** | 1.51 → **2.89** | 3.96 | 281 |
| bird | 1852 | 47 | 60.6 → **8.2** | 1.39 → **2.83** | 3.96 | 115 |
| horse | 2146 | 44 | 55.2 → **2.7** | 1.45 → **2.73** | 3.97 | 333 |
| fish | 3526 | 25 | 67.4 → **10.1** | 1.33 → **2.84** | 3.93 | 198 |
| dragon | 2561 | 76 | 73.0 → **23.7** | 1.27 → **2.34** | 3.51 | 111 |
| kaiju | 1571 | 45 | 47.9 → **4.6** | 1.52 → **2.96** | 3.97 | 204 |
| snake | 995 | 24 | 19.0 → **3.3** | 1.81 → **3.15** | 3.98 | 13 |
| spider | 924 | 43 | 58.4 → **15.5** | 1.42 → **2.24** | 3.98 | 141 |

  - 0 fallback vertices on every template — no vertex needed the Euclidean guess.
  - **The bone mask had to reproduce the legacy skip set exactly** (`name === 'root'`, or childless *and* named `leaf`/`tip`) or the comparison is not like-for-like. A structural definition ("no Bone child") looked equivalent and was not: it excluded 8 `wing_feather_*` bones on the bird that legacy weights, giving 39 bones against legacy's 47.
  - The `raw mean` column counts any non-zero influence, the threshold the legacy baseline used, so it is the directly comparable figure; the `mean influences` column counts influences above 1% and is the honest one.
  - Dragon (23.7%) and spider (15.5%) improve least; both are the busiest rigs relative to their vertex count (76 bones / 2561 verts, 43 / 924). Worth revisiting at P4-4.
- [x] **P1-9** The three legacy weight correctors are **not ported**, and P1-8 is the justification *(2026-08-30)*
  - They are not deleted from `legacy/` — that tree must stay runnable as the A/B baseline (`test.md` §9). The decision is that no equivalent exists in `m2m-core` and none will be added.
  - They existed to patch the Euclidean nearest-bone failure: `ArmWeightCorrector` for arms near the ribcage, `ExtremityWeightCorrector` for fingers grabbing knuckles, `HeadWeightCorrector` for the head/neck boundary, plus `WeightSmoother` for the seams single-bone assignment leaves. Geodesic distance removes the cause, so there is nothing for them to fix — every template improves without any per-body-part correction.
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

## Asset issues found (not solver bugs)

- **`rig-shark.glb`'s `back_fin_2_l/r` bones sit outside `model-shark.glb`.** Indices 13 and 17, a mirrored pair at x = ±0.451, z = −1.83. Found by `unreachable_bones()` during P1-8 and pinned as an expected value in the A/B so a *new* unreachable bone anywhere fails the test. Harmless in the app — the user fits the skeleton before binding — but the shipped template does not fit its own model there.

## Blocked / needs user input

- **`references/` licensing** — the 7 Mixamo FBX files are gitignored for now: royalty-free to use but not CC0, and this repo licenses all art as CC0. Confirm whether to commit them anyway, keep them local, or replace with CC0 equivalents (R-5).

## Deferred with reason

- **Windows/Linux builds** — macOS-native first per `project_context.md` non-goals. Do not add cross-platform abstraction speculatively.
- **Neural rigging as core** — ADR A5: ~1.5 GB RAM + ORT-from-source conflicts with the resource budget. Revisit at P4-6.
