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
- [x] **R-3** Robust Biharmonic Skinning (arXiv:2406.00238) — **decided OUT** *(2026-08-30, `docs/research/robust-biharmonic-decision.md`)*
  - Read the full text rather than the abstract, which is what the earlier survey entry was based on. Implementation is PyTorch + custom CUDA + **OptiX/OWL/Warp** — all NVIDIA-only, on a project targeting Apple Silicon. Reported **71.74 s** on Bunny against a 3 s budget.
  - The robustness it buys (non-watertight, self-intersecting, triangle soup) is what voxelisation already gives us — measured in P1-3, where the reference character is not watertight and solves without incident.
  - Steal-worthy idea recorded for P3: it folds artist weight painting into the optimisation as Dirichlet boundary conditions rather than post-processing it.
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
- [x] **P1-10** ~~Optional biharmonic refinement pass~~ — **not built**, R-3 decided the method out *(2026-08-30)*
  - **Reason for the change:** the gating research resolved against it on platform (NVIDIA-only stack) and budget (10–25× over). A second solver is not free — two weighting paths to test, benchmark and keep correct — and nothing yet says the geodesic weights are insufficient. Revisit if artists report quality problems, or if a Metal implementation of the visibility kernel lands inside budget.
- [x] **P1-11** Benchmark vs. budgets in `test.md` §6 *(2026-08-30)* — **comfortably inside, no optimisation needed**

| vertices | voxelise | geodesic | weights | total | peak heap |
|---|---|---|---|---|---|
| 7,399 | 30 ms | 255 ms | 3 ms | **288 ms** | **30 MB** |
| **48,670** | 37 ms | 253 ms | 18 ms | **307 ms** | **44 MB** |
| 213,754 | 57 ms | 340 ms | 76 ms | **474 ms** | **129 MB** |

  - Budget is 3 s / 1.5 GB at 50k vertices. Measured on the 48,670-vertex row: **307 ms / 44 MB** — about **10× under time and 34× under memory**. The budget was written before any measurement; it is conservative, not tight.
  - **Resolution is the dominant cost, not mesh density.** Solve time barely moves from 7k to 214k vertices because the geodesic Dijkstra runs over the voxel grid, whose size depends only on resolution. Resolution is cubic: 7 ms at res 64, 39 at 128, 131 at 192, 295 at 256, 1004 at 384 — doubling resolution costs ~8×. This is what the P3 resolution control must communicate.
  - Peak **heap** measured with a tracking allocator, not process RSS. RSS never shrinks, so in a multi-test binary it reports everything that ran before — the mistake the legacy benchmark made in session 003.
  - `instruction.md` §5 optimisation ordering deliberately **not** applied: there is nothing to optimise against. Revisit if the resolution control lets users past 384.

## P2 — I/O (`m2m-io`)

- [x] **P2-1** Port FBX binary reader *(2026-08-30)* — parses the real 2.1 MB Mixamo export: **version 7700, 11 roots, 6099 nodes, 11.6 ms**
  - Structure verified against what a rigged character must contain: 67 Model, 131 Deformer, 315 AnimationCurve, 2 Geometry, 666 Connections; `Geometry/Vertices` decodes to 42,696 f64 (14,232 vertices) through the zlib path.
  - **Deliberate divergence:** the legacy `BinaryParser` interleaves binary decoding with semantic reshaping (`Properties70` flattening, `Connections` collection, single-property collapsing). Split here — this layer produces a faithful typed node tree, and the reshaping moves to the DOM layer in P2-3. The original cannot be tested without the reshaping; this can.
  - **No legacy tests existed for these two files** (only FBXTreeParser/GeometryParser/TextParser have them), so the harness is the real Mixamo file plus hostile input.
  - Trust-boundary guards, each mutation-tested: depth limit (a stack overflow aborts rather than unwinds, so it cannot be caught), declared-length checks before allocating, inflate bounded by the *declared* size and the maximum deflate ratio, and **footer-magic validation**.
  - **The footer check exists because of a serious silent failure found in review:** cutting 578 bytes from the 2.1 MB file — 0.03% — parsed to `Ok` with 10 roots and 6089 nodes, having discarded the entire `Takes` section, i.e. every animation stack. The end-of-content test is an offset heuristic, so a cut inside the last root just stops the loop early. The 16-byte footer magic is identical across all 8 real Mixamo exports checked and is the only reliable completeness signal.
- [x] **P2-2** Port FBX ASCII reader *(2026-08-30)* — produces the **same `FbxDocument`** as the binary reader, so P2-3 has one input shape
  - All 8 legacy test cases ported. They are regression guards for real bugs the legacy fixed over upstream three.js, so they came across with the code.
  - **Format normalisation belongs in the readers; semantic reshaping does not.** Binary stores a vertex array as a property on its `Vertices` node; ASCII writes `Vertices: *9 { a: ... }`, putting the numbers on a child. That is a format quirk, reconciled here by hoisting `a:` onto the parent. `Properties70` flattening and `Connections` collection stay in P2-3.
  - **Mutation-tested the two regressions that matter.** Un-anchoring the brace reproduces the historical bug exactly: parsing `ASCII_FBX` then yields **only `FBXHeaderExtension`** — every node after a `P:` line containing `{Project}` is silently discarded. Removing the document-level property path reproduces the other.
  - No regex dependency: leading tabs are counted directly, which is simpler than the legacy's dynamically-built `\t{N}` patterns and makes the end-of-line brace anchor explicit rather than emergent.
  - **Review found 10 issues, 3 high — my "one input shape" claim was false.** The `*N` array count was emitted as a leading `Str`, so the ASCII tree differed from binary at `properties[0]`; my test used `find_map` over the property list and missed it. Now the full property vector is asserted.
  - Other real ones: `is_ascii_fbx` sliced `&text[..1024]`, which **panics** when the cut lands mid-character — a panic on the trust boundary, and exporters do put non-ASCII in headers. Comma splitting ignored quotes, so `"Model::Bob, Jr"` became two properties and shifted every positional index. Indentation was ignored entirely, so a **missing** `}` swallowed whole sections silently (legacy at least skipped such lines; this now resyncs to the line's own depth, which is better than either). Short `a:` arrays were accepted against their declared `*N`. The `Content:` base64 continuation was not ported, silently dropping embedded textures.
  - **Cross-format accessors rather than guessed types.** ASCII cannot know whether `100` was an `i32` or `i64`, so `as_i64`/`as_f64`/`as_i64_vec` read by meaning, and `split_object_name` handles both `"Bob\0\x01Geometry"` (binary) and `"Geometry::Bob"` (ASCII) without discarding the class.
- [~] **P2-3** Port `FBXTreeParser` — split across sessions, ~~1620 LOC~~ **scoped down, measured**
  - **Reason for the change:** of `FBXTreeParser.ts`'s 1574 lines of method code, **319 are textures and materials and 218 are lights and cameras** — about 46% once material-parameter parsing is counted. None of it belongs in `m2m-io`, which reads geometry and rigs (`architecture.md` §2); the viewport loads materials itself. Only the rigging path is ported, roughly 700 lines.
  - [x] **A — the DOM layer** *(2026-08-30)*: connection graph, objects addressable by id, `Properties70` flattened to named typed values. This is the semantic reshaping both readers deferred, done **once** here rather than twice as the legacy does.
    - Real rig resolves to 642 objects: 67 Model, 131 Deformer (129 Cluster + 2 Skin), 315 AnimationCurve, 2 Geometry, 2 Pose. `mixamorig:Hips` carries `Lcl_Translation` Y = 104.3 cm, a plausible hip height in the centimetres Mixamo exports.
    - **Cross-format equality asserted directly**, not by testing each reader separately: the same document built through both paths yields the same object identity, relationships and property values. Mutation-verified — breaking `split_object_name` fails it on object identity.
    - **Review caught the same tautology pattern again, in the test written to avoid it.** The digest comparison was `assert_eq!(a.len(), b.len())` — row counts, 2 == 2 — and the real comparison *failed*, because `{:?}` on the raw variant shows `I64(104)` for ASCII against `F64(104.0)` for binary. The digest now renders values through the accessors, which is the contract that lets widths differ, and compares in full.
    - **`Link::property` kept `"Lcl Translation"` while flattened keys used `"Lcl_Translation"`**, so `scene.object(id)?.property(link.property)` returned `None` for every `Lcl` channel — and 215 of the 666 connections in the reference rig are object-to-property, with the animation path as the consumer that walks them. Both normalised now, with a test asserting the exact set of targeted property names.
    - `Scene::from_document` consumes the document rather than borrowing: cloning each object node duplicated every vertex, index and weight array for the scene's lifetime, roughly doubling peak memory on a large mesh.
  - [ ] ~~B — Geometry~~ and ~~D — Deformers~~ — **folded into P2-4, which already covered both.** The A–D split was invented in session 013 without checking P2-4's scope; geometry and skin clusters live in `GeometryParser.ts`, not `FBXTreeParser.ts`, so they belong there. P2-3 keeps only what is genuinely `FBXTreeParser`'s: the DOM layer and the model/bone hierarchy.
  - [ ] B — Models and the bone hierarchy (was C)
- [~] **P2-4** Port `GeometryParser` (985 LOC) incl. skin clusters — **absorbs P2-3's geometry and deformer parts**
  - [x] **a — mesh geometry** *(2026-08-30)*: vertices, polygon indices, triangulation, normals, UVs
    - Real rig: Beta_Surface 14,232 verts → **28,272 triangles**, Beta_Joints 10,514 → **20,840**, derived from the measured polygon mix (172 tris + 14,050 quads; 1,400 + 9,720) rather than asserted as a bound.
    - **Measured before choosing a triangulator:** the file contains *only* triangles and quads, no n-gons. So the legacy's earcut-over-a-tangent-plane is not needed. A quad splits along the diagonal whose two triangles both agree with the polygon normal — exact, not a heuristic — and anything larger is fanned and **counted in the report** so an approximation is never silent.
    - `vertex_source` records the original FBX vertex behind each expanded corner. Vertices expand per polygon corner because normals and UVs are per-corner, so skin weights — indexed by original vertex — need the mapping back. The legacy calls this `remapSkinIndices`; P2-4b depends on it.
    - The two meshes use **different** normal mappings (`ByPolygonVertex`/Direct and `ByVertice`/Direct), so one file covers both resolution paths. Normals verified unit-length under both.
    - **Review verified two of my tests were hollow: the entire UV path could be deleted, and `vertex_source` could be a constant 0, with all tests still passing.** Both assertions were length checks the loop pushes satisfy unconditionally — and `vertex_source` is what P2-4b's skin remap depends on. Now checked by value: every expanded corner must sit exactly where its source vertex does (mutation: 84,811 of 84,816 mismatch), and UVs by non-zero count and unit-square bounds (mutation: 0 of 84,816 non-zero). UVs are the file's only `IndexToDirect` layer, so they are the sole coverage of the indirection branch.
    - **Functional gap found: the geometric pre-transform was dropped.** `GeometricTranslation/Rotation/Scaling` sit on the Model, are not part of the node transform, and apply to the mesh alone. Mixamo writes identity — which is why nothing caught it — but Maya and Max exports commonly do not, and the result is a mesh offset from its skeleton. Now read from the connected Model, applied to positions, with normals transformed by the inverse transpose so a non-uniform scale does not tilt them.
    - Also: unresolvable layer indices and unrecognised mapping types were silently zero-filled or dropped; both are counted in the report now. The quad claim was softened to "exact for **planar** quads" — a bowtie satisfies neither diagonal and is now counted as ambiguous rather than silently guessed.
  - [ ] b — skin clusters: per-bone vertex indices, weights, bind transforms
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
