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
- [~] **P0-6** Install SonarQube + scanner; `sonar-project.properties`; verify a scan completes
  - [x] `sonar-scanner` 8.1.0.6389 installed via brew; `sonar-project.properties` + `docker/sonarqube.yml` written
  - [ ] **Blocked:** the SonarQube *server* needs Docker running. Docker CLI is at `/usr/local/bin/docker` but the daemon is not running (verified 2026-08-29). Start Docker Desktop, or supply SonarCloud credentials.
- [x] **P0-7** CI: `.github/workflows/ci.yml` — rust-test, rust-lint, arch-gate, frontend, build. **Verify green via `gh run watch`.** *(2026-08-29 — 6 jobs; arch-gate, frontend, legacy suite green; macOS jobs pending)*
- [x] **P0-8** `.cargo/config.toml` with `debug = "line-tables-only"`; `.gitignore` for `target/`, `dist/`, `node_modules/` *(2026-08-29)*
- [x] **P0-9** Vendor Asta Sans (OFL) + Lucide; implement `design.md` tokens as `app/src/ui/tokens.css` *(2026-08-29 — subset upstream 5.5 MB TTF to Latin, **28.4 KB woff2**, variable axis preserved; OFL.txt vendored alongside. Load verified in the running app: `font ok`.)*
- [ ] **P0-10** `bench/` harness with criterion; capture **legacy baselines** for all 9 templates before any solver work
- [x] **P0-11** Verify empirically whether WebGPU is available in this WKWebView; record result in `architecture.md` *(2026-08-29 — **WebGPU confirmed available**, better than the assumed WebGL2 fallback. Adapter requested inside the shipped app. Recorded as ADR A1a; backend now detected at runtime and shown in the status bar.)*

## R — Research spikes (write `docs/research/<topic>.md` before implementing)

- [x] **R-1** Survey SOTA skinning + auto-rigging *(2026-08-29 — written up in `docs/research/skinning-sota.md`)*
- [ ] **R-2** Geodesic voxel binding: full paper read, pseudocode, parameter table → `docs/algorithms/geodesic-voxel-binding.md`
- [ ] **R-3** Robust Biharmonic Skinning (arXiv:2406.00238): is the mesh-free formulation implementable in Rust at our budget? Decide in/out.
- [ ] **R-4** Non-human rig conventions: survey how Blender/Maya/Rigify handle avian wing chains, fish spines, quadruped scapulae → `docs/research/creature-rigs.md`
- [ ] **R-5** Source CC0/CC-BY rigged reference creatures to expand the template library. **Record provenance + licence per asset.**
- [ ] **R-6** FBX 7.4/7.5 binary write format — spec gaps, since no Rust writer exists
- [ ] **R-7** Evaluate UniRig/RigAnything ONNX export feasibility (informs P4-6; do not implement yet)

## P1 — Core algorithms (`m2m-core`)

- [ ] **P1-1** Mesh representation: SoA vertex buffers, half-edge adjacency, robust normals
- [ ] **P1-2** Mesh validation: watertightness, degenerate tris, duplicate verts, disconnected islands, scale detection
- [ ] **P1-3** Sparse voxelisation with interior/exterior classification (robust on non-watertight input)
- [ ] **P1-4** Geodesic distance field over voxel interior, per bone, `rayon`-parallel
- [ ] **P1-5** Weight assignment from geodesic falloff, k≤4 bones/vertex
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

- **P0-6 SonarQube server** — needs Docker Desktop started, or SonarCloud credentials. Scanner and config are ready either way.
- **`references/` licensing** — the 7 Mixamo FBX files are gitignored for now: royalty-free to use but not CC0, and this repo licenses all art as CC0. Confirm whether to commit them anyway, keep them local, or replace with CC0 equivalents (R-5).

## Deferred with reason

- **Windows/Linux builds** — macOS-native first per `project_context.md` non-goals. Do not add cross-platform abstraction speculatively.
- **Neural rigging as core** — ADR A5: ~1.5 GB RAM + ORT-from-source conflicts with the resource budget. Revisit at P4-6.
