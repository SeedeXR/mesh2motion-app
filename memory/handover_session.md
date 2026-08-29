# Session Handover Log

Newest entries at the top. Every session appends one entry before exiting.
Timestamps are local (macOS, `date "+%Y-%m-%d %H:%M:%S"`).

---

## Session 006 — 2026-08-29

**Ended:** 2026-08-29 19:15:03
**Focus:** P1-4 geodesic distance field — the core of the whole solver

### Completed
**P1-4.** Dijkstra over a compact graph of non-exterior voxels, 26-connected
with true Euclidean step lengths, one bone per `rayon` task. 46 tests.

### Memory was the design constraint
A distance field per bone over the whole grid is 66 x 3.4M x 4 = **900 MB**,
past the budget. Two things avoid it: only non-exterior voxels participate
(201k of 3.4M, a 17x reduction — the exterior is what would have dominated),
and only distances **at vertices** are retained. Result: **1.9 MB**, and
**190 ms** for 7399 verts x 66 bones at resolution 256.

Surface voxels are included in the graph, not just interior ones — vertices sit
on the surface, so excluding them would strand every vertex.

### The measurement that justifies the project
For each vertex, the bone Euclidean distance picks (what
`WeightCalculator.ts:71-80` does) vs the bone geodesic distance picks:

| | |
|---|---|
| dominant bone changes | **1080 of 7399 (14.6%)** |
| path ratio p50 / p90 / p99 | 1.06 / 1.51 / 3.33 |
| **worst** | **19.4x** |

One vertex in seven is assigned to a different bone. The 19.4x case is "hand
near the hip" exactly: the Euclidean-nearest bone is nineteen times further
away measured through the body. **And this is a T-pose model** — the case most
favourable to Euclidean. A-pose should be worse, which is the quantitative
basis for O8.

### Fixture mismatch caught by the feature itself
First run reported **65 of 66 bones unreachable**. Not a solver bug: I had
paired `rig-human.glb` with `human-small.glb`, and the rig is **2.19x** larger —
`human-small` is a scaled-down test asset. Checked the bounds rather than
assuming either way. Exported `model-human.glb` as `human-template.bin`, the
model the rig was actually authored for, and the matched pair reports 0
unreachable bones and 0 stranded vertices.

Pleasingly, this was `unreachable_bones()` working: "your skeleton is outside
your mesh" is the most common rigging mistake and the UI must surface it. Kept
as a deliberate test, now with the complementary half so it cannot pass
vacuously.

### Code review — 7 findings, all resolved
- **Resolution floor.** Review argued surface voxels plus 26-connectivity let
  paths leak across sub-voxel gaps, defeating the central claim. Measured it
  myself: the threshold is ~1.5 voxels (leaks at 1.07, separates at 1.60), not
  where the reviewer estimated, but the point stands. This is inherent to voxel
  methods, so it is now documented with the measured table and pinned by a
  test. At the default resolution on a 1.75 m human it is a ~1 cm floor: an
  A-pose arm at 2-5 cm is fine, an arm touching the body is not.
- **Stranded vertices inflated the headline statistic.** `min_by` over an
  all-infinite row returns the last bone rather than failing, so unreachable
  vertices counted as "dominant bone differs". The template has none so 14.6%
  was honest, but on a model with islands the assertion could have passed on
  noise alone. Now skipped and counted separately.
- `Visit`'s derived `PartialEq` disagreed with its manual `Ord` — harmless for
  `BinaryHeap`, silently wrong in a `BTreeSet`. Tie-broken on node index.
- The seed-sampling comment claimed half-voxel spacing "cannot skip a voxel".
  It cannot skip more than one voxel per axis, which is weaker; corrected, with
  the exact alternative named.
- The memory assertion was a tautology computed from the counts it printed, and
  the measured timing was never asserted. Replaced with a loose 10 s ceiling.
- The doc comment claimed the field was "scratch, reused per worker thread"; it
  is allocated per bone and the transpose doubles peak. Comment corrected.

### Next session starts at
**P1-5** — weight assignment from geodesic falloff, k <= 4 bones per vertex,
with the nearest-bone fallback for stranded vertices that P1-4's
`unreachable_vertices()` now identifies.

---

## Session 005 — 2026-08-29

**Ended:** 2026-08-29 18:43:54
**Focus:** P1-3 sparse voxelisation — first piece of the geodesic pipeline

### Completed
**P1-3.** Conservative triangle-AABB rasterisation (Akenine-Möller 13-axis SAT),
then a 6-connected exterior flood fill from the padded grid boundary; interior
is whatever neither reaches. 33 tests total across the crate.

### DEFAULT_RESOLUTION = 256, measured not guessed
Release-build sweep on the real character fixture:

| resolution | surface | interior | interior/surface | volume | time | memory |
|---|---|---|---|---|---|---|
| 32 | 795 | 66 | 0.08 | 0.0011 | 0.3 ms | 9 KB |
| 128 | 13357 | 15228 | 1.14 | 0.0039 | 4 ms | 421 KB |
| **256** | 54559 | 146692 | **2.69** | **0.0047** | **24 ms** | **3.1 MB** |
| 384 | 124137 | 525375 | 4.23 | 0.0050 | 79 ms | 10.3 MB |

**The interior/surface ratio is the deciding metric**, not raw counts. Below
~128 the grid is shell-dominated — thin limbs are entirely surface with no
interior between them, so the P1-4 geodesic field would have almost nothing to
propagate through. My first instinct was to read the low 1.1% fill at res 32 as
a bug; sweeping showed it is the documented thin-feature limit instead.

### Physical validation, not just internal consistency
Interior volume at 256 is 0.0047 units³. The figure is 0.764 units tall, so a
1.75 m human implies 2.29 m/unit and a volume scale of 12.0 → **56 litres**. A
60 kg person displaces roughly 60 L. This is now a test assertion: it checks the
pipeline against physical reality rather than against itself.

### The 61-component requirement is met
All components share one grid, so spatially-nested islands connect in voxel
space even though their surfaces never touch. Tested directly: a small cube
fully inside a large one validates as 2 surface components, and the inner
cube's centre classifies as Interior.

### Leak behaviour, tested both ways
The real mesh is **not** watertight (26 boundary edges, 1 non-manifold) and
still encloses volume at every resolution — conservative rasterisation seals
holes smaller than a voxel. A synthetic box with a whole face removed leaks
completely (interior 0). Both are asserted, so the boundary of the method's
robustness is pinned rather than assumed.

### The worst bug so far, and why my tests missed it
Code review flagged that an axis-aligned cube might not rasterise. Verified it:
**a plain unit cube produced NO SHELL AT ALL at resolutions 13, 18 and 20, and
leaked completely at 16 more.** My tests used 8/16/24/32 and passed by luck —
those give power-of-two voxel sizes.

Organic geometry hides this entirely, which is why the real-mesh test was green:
a character never lands exactly on a voxel plane. A cube's faces do,
*systematically*, because the grid origin is derived from the mesh bounds. So
"conservative rasterisation seals the shell" was false for every box, prop,
flat sole, or ground plane — and I had written it into the docs as established.

Three independent causes, all required:
1. **The rasterisation AABB excluded the voxel actually containing the face.**
   `coord_of` and the box centre round independently: at resolution 20 the voxel
   size is 0.050000001, `coord_of(0.0)` returns voxel 1 whose box starts at
   2e-9, and the face at x=0 really sits in voxel 0 — never tested. This was the
   primary cause and the review had *not* diagnosed it; I found it by printing
   the actual coordinates rather than trusting the diagnosis.
2. **No epsilon in the overlap test**, so exact touching came down to one ulp.
   Voxel boxes are now tested very slightly enlarged (1e-3 of a voxel).
3. **World-space rasterisation lost precision** for a small model far from the
   origin. Found by sweeping: only the (scale 0.01, offset 123.456) combination
   failed — f32's ulp at 123 is ~1e-5, just 30x below that voxel size.
   Rasterisation now runs in grid-local coordinates, so precision depends on the
   model's own extent rather than where it sits in the artist's scene.

Regression test sweeps 3 scales x 4 offsets x 3 rotations x 41 resolutions =
**1476 cases, all sealed**. Padding also went 1 -> 2, since the AABB widening
can consume a single layer and leave the flood fill no seed.

Real-mesh numbers barely moved (surface +50, interior -20), which confirms the
bug was specific to axis-aligned geometry.

### Also from review
Scale-relative degenerate-axis floor (was an absolute 1e-20, which on a
sub-1e-5-scale mesh would make every axis "degenerate" and inflate the shell to
the whole AABB); `checked_mul` on the voxel count (a large resolution wrapped
usize and then panicked out of bounds); triangle-normal tested first since it
rejects the most candidates.

### Repeated mistake
Wrote `!(longest > 0.0)` again — the exact clippy pattern I fixed in `mesh.rs`
last session. Fixing an instance is not the same as learning the rule.

### Next session starts at
**P1-4** — geodesic distance field over the voxel interior, per bone,
`rayon`-parallel. The grid API it needs (`index`, `state`, `center`,
`coord_of`, `dims`) is already in place.

---

## Session 004 — 2026-08-29

**Ended:** 2026-08-29 18:18:53
**Focus:** P1-1 mesh representation, P1-2 mesh validation — first real solver code

### Completed
- **P1-1** mesh representation · **P1-2** mesh validation. 22 tests (19 unit + 3 integration), clippy and fmt clean.

### Ladder decisions (things deliberately NOT built)
- **No half-edge structure**, despite P1-1 naming one. Geodesic voxel binding runs on the voxel grid, not the mesh graph; the only consumer of adjacency is validation, which needs an edge→face count, not a half-edge. Building one would have been speculative.
- **No scale detection**, despite P1-2 naming it. The solver normalises by the bounding-box diagonal anyway (invariant 7), so guessing units is a UI concern. The raw diagonal is reported instead of an invented `ScaleHint` enum with thresholds I could not justify.
- **No normals** until something needs them.
- Hand-rolled 20-line union-find rather than a crate — ladder rung 5.

### The finding that matters: real meshes are not one island
`legacy/static/test-files/human-small.glb` — all 3 meshes merged with world
transforms baked, 8691 verts / 13721 tris — exported to a binary fixture so
`m2m-core` can be tested on real geometry without an I/O dependency (it does no
I/O by design, and `m2m-io` does not exist until P2):

| | |
|---|---|
| connected components | **61** |
| duplicate (seam-split) vertices | 1698 |
| boundary edges | 26 |
| non-manifold edges | 1 |
| watertight | **no** |

I asserted "a human body should be one island" and it failed. It was the
assertion that was wrong. A character is eyes, teeth, tongue, lashes and
clothing as well as a body.

Verified it is not an epsilon artefact by sweeping: **116** components unwelded,
a **stable band of 61** from 1e-7 to 1e-5 of the diagonal, 59 at 1e-4, and
collapse beyond that. `DEFAULT_WELD_EPSILON_RATIO` is the log-centre of that
band, **1e-6**, with a test pinning the band so the default cannot drift onto a
slope. The sharpest over-welding signal turned out to be the degenerate-triangle
count: at 1e-3, welding has collapsed **2890 of 13721 real faces** into slivers.

**Consequence for P1-3/P1-5, recorded in the algorithm doc and todo:** geodesic
distance cannot propagate between disconnected islands, so a naive
implementation gives eyes and teeth **zero weight from every bone** and they
detach. All components must voxelise into one shared grid (spatially-nested
islands then connect in voxel space — another reason the method is voxel-based),
and anything still isolated must fall back to nearest-bone and be flagged.
Never leave a vertex unweighted.

### Bug found by the tests
`weld_map` overflowed on a denormal epsilon: `1.0/f32::MIN_POSITIVE` saturates
every cell coordinate to `i64::MAX`, and the 27-cell neighbour scan then
overflowed adding 1. Fixed with an early exit for non-finite/non-positive
epsilon plus saturating offsets. Also corrected a test whose premise was wrong:
*exactly* coincident vertices weld at any positive epsilon because their
distance is exactly zero — only a disabled epsilon skips welding.

### Code review — 9 findings, all resolved
The strongest review yet; three findings were serious.
- **HIGH** the degeneracy test `area2 <= f32::EPSILON` was an **absolute**
  threshold on a scale-dependent quantity (cross-product magnitude, mesh-units
  squared). The reviewer measured only ~20x margin on the fixture: re-exporting
  the same model at 1/5 scale would have flagged real faces as degenerate and
  moved both the component and boundary-edge counts. Now relative to the
  squared diagonal, with a scale-invariance test across 1e-2 .. 1e3.
- **MED** degenerate faces were dropped from edge counting entirely, so one
  decimation sliver in a closed mesh made its neighbours' shared edges look
  used once — phantom holes, and `is_watertight()` false for a mesh with none.
  Fixed by splitting the two cases: a face with a **repeated corner** is not a
  face and is excluded from topology; a **sliver** (distinct corners, no area)
  is reported but keeps its edges.
- **MED** my `DEFAULT_WELD_EPSILON_RATIO` justification was **circular**. The
  fixture's duplicates are bit-exact, so every epsilon gave the same answer;
  the "36 components below the plateau" data point came from the *disabled*
  path, not from a too-small epsilon. Re-derived honestly on the full model,
  where a real band does exist, and the constant moved 1e-5 -> 1e-6 because
  1e-5 sat at its upper edge.
- **MED** `dump-fixtures.ts` took the first mesh and ignored world transforms —
  the same trap `model-shark` set earlier. It now asserts and merges: the
  fixture went from **1761 verts (20% of the model) to 8691**, and every number
  above changed as a result. Documented weld semantics as
  representative-based and order-dependent rather than the transitive "within
  epsilon" the doc had claimed; guarded subnormal epsilon (finite, positive,
  but `1.0/eps` is infinity); checked arithmetic in the fixture parser;
  replaced a vacuous assertion with the real degenerate count.

**Lesson:** every one of my own measured claims this session was wrong in the
same direction — measured on a fraction of the data and stated as if whole.

### Process bug fixed: false-green CI
`gh run list --limit 1` returned the **previous** commit's run — already green —
because a fresh push takes a few seconds to register. I nearly recorded that as
this commit's CI result. `session_start.md` §6 now requires matching the run to
HEAD by SHA. Actual run for 8a78fa27 is 33260571138: all 6 jobs green, verified
against the SHA.

### Next session starts at
**P1-3** — sparse voxelisation with interior/exterior classification, carrying
the 61-component requirement above.

---

## Session 003 — 2026-08-29

**Ended:** 2026-08-29 17:30:08
**Focus:** P0-6 SonarQube (unblocked), P0-10 legacy solver baselines

### Completed
- **P0-6** — Docker went live, so SonarQube 26.8.0 community is running via `docker/sonarqube.yml`. Scan completed: **quality gate OK, 0 issues**. Community Edition has no Rust analyser, so Rust stays gated by `clippy -D warnings`; Sonar covers the TypeScript frontend.
- **P0-10** — legacy solver baselines captured for all 9 templates → `bench/baselines/legacy-solver.json`.

### The baseline number that matters
**68% of 22196 vertices carry only ONE bone influence.** Mean influences per vertex range 1.13 (human) to 1.81 (snake), against a GPU limit of 4. Smooth deformation needs 2-4 influences near joints, so this is the rigid-assignment defect quantified — and it is the metric P1-8's A/B must move. Human is worst at 87%.

Also: **0 unnormalised vertices** across all templates. The legacy normaliser is correct and P1 must preserve that.

### Built
- `legacy/bench/glb-headless.ts` — loads GLB in Node by stripping textures from the JSON chunk and rewriting the container. GLTFLoader parses geometry and skeletons fine headlessly, but material loading reaches for canvas/ImageBitmap decoding Node lacks. Stripping the JSON keeps this independent of GLTFLoader internals.
- `legacy/bench/solver-baseline.ts` + `vitest.bench.config.ts` + `npm run bench`. Deliberately a **separate** vitest config so benchmarks never run in the legacy CI job.

### Two bugs I found in my own harness before committing
Both would have produced confident, wrong baselines:
1. **`firstGeometry()` took only the first mesh.** `model-shark` ships as two meshes (1948 + 1578 verts), so the fish baseline silently measured 55% of the geometry and reported it as the whole model. Now solves every mesh.
2. **`peakRssMb` measured cumulative process RSS, not per-solve cost.** The tell was the numbers increasing monotonically regardless of mesh size — a 924-vertex spider "used" more than a 7399-vertex human. `global.gc()` is a no-op without `--expose-gc`. Now measures `heapUsed` delta with a forced collection, and the figures scale with vertex count as they should.

**Lesson:** the harness passed green in 317 ms and I nearly accepted it. The instinct that saved it was checking whether the *numbers were physically plausible*, not whether the test passed. Benchmarks need their output sanity-checked, not just their exit code.

### Code review — 10 findings, all resolved
The agent independently found both bugs above (deducing from the committed JSON
that the artifact came from a fixed version), confirmed the **GLB binary rewrite
is correct** by parsing all 18 GLB assets, and found six more:
- **MED** the benchmark caught every exception, reported green, and overwrote
  `legacy-solver.json` with error rows — destroying the artifact it exists to
  protect. Now throws and writes nothing on failure. *Verified by pointing a
  template at a missing rig: run fails loudly, baseline preserved.*
- **MED** `global.gc?.()` was always a no-op without `--expose-gc`; the npm
  script now passes it.
- **LOW** `nonZero <= 1` counted **unweighted** vertices as single-influence,
  inflating the exact metric P1's A/B keys on. Now `=== 1`, with
  `zeroInfluence` tracked separately (currently 0 across all templates).
- **LOW** empty weights gave `0/0` → `NaN` → `null` in JSON, reading like a
  real measurement; and `hashDominantBones` returned the bare FNV seed, which
  looks like a legitimate fingerprint. Both guarded.
- **LOW** median indexed by the `RUNS` constant rather than the array length.
- **LOW** `stripTextures` missed texture slots nested in material extensions
  (`KHR_materials_specular`, `_clearcoat`, `_sheen`). Not triggered by today's
  assets — the reviewer checked — but any re-export with a clearcoat or
  specular-colour map would have broken the bench. Now recurses.
- **LOW** only the GLB magic was validated, so a truncated file reached
  `JSON.parse` on binary. Chunk type and length are now checked.

### Notes
- jsdom breaks GLB loading — `ArrayBuffer` identity fails across realms. Benchmarks run in the `node` environment.
- SonarQube credentials are local-only; token is not committed.

### New requirement from user: A-pose **and** T-pose humans
Recorded as objective **O8** and roadmap section **P3-P**. Investigated the
current state first: A-pose today is a workaround, not support —
`human-a-pose.glb` is a *test file* special-cased into the model dropdown
(`legacy/src/lib/DOMUtilities.ts:445-450`), `ArmWeightCorrector` + the
arm-plane-offset slider exist precisely because A-pose arms hang near the
ribcage, and `ArmExtensionControl` is a manual percentage nudge between poses.

Decomposes into four problems, only one of which the current plan already
solves: weights (geodesic solver handles it natively), pose detection,
pose-aware skeleton fitting, and — the real work — retargeting across a pose
mismatch, which needs `source_rest⁻¹ · target_rest` per bone rather than
assuming a shared rest pose. `RetargetUtils.capture_bone_rest_transforms`
already recovers what that needs.

### Attempted and backed out: A-pose baseline
Added `human-a-pose` to the benchmark, then removed it. The harness applies the
template rig with **no fitting step**, and `rig-human.glb` is a T-pose rig
(`hand_l` at world x=0.75) while the A-pose mesh has arms down (x spans ±0.62
vs the T-pose mesh's ±0.97). That row measured a rig/mesh mismatch while
reading like a valid baseline — worse than having no row.

Also built and removed an `armBleed` metric. Two findings worth keeping:
1. **The aggregate influence metrics cannot detect the A-pose defect.** Measured:
   85% vs 87% single-influence, 1.151 vs 1.132 mean influences — essentially
   identical. The defect is *which* bone claims a vertex, not how many
   influences it has. P1-8 needs a targeted metric.
2. The metric returned 0 because |x| < shoulder-x is a narrow torso column that
   A-pose arms never enter. Shipping it would have given a confident,
   meaningless zero.

### Next session starts at
**R-2** — geodesic voxel binding algorithm doc (`docs/algorithms/`), then **P1-1**
(mesh representation) to begin the solver.

---

## Session 002 — 2026-08-29

**Ended:** 2026-08-29 17:00:00
**Focus:** P0-11 WebGPU verification, P0-9 font vendoring

### Completed
- **P0-11** — **WebGPU confirmed available** in this WKWebView (macOS 26.6.2). Measured by requesting a real adapter inside the shipped app, not by feature-sniffing `navigator.gpu`. This is *better* than ADR A1 assumed (WebGL2 fallback): the viewport reaches Metal through WebGPU with no native renderer. Recorded as **ADR A1a**.
- **P0-9** — Asta Sans vendored. Upstream is a 5.5 MB variable TTF with full Hangul coverage; subset to Latin + punctuation gives **28.4 KB woff2** (99.5% smaller) with the 300–800 weight axis intact. `OFL.txt` vendored alongside. No CDN dependency.

### Added
- `app/src/viewport/backend.ts` — render backend detection, probing for a real adapter
- `report_startup` Tauri command — logs render backend + font load status at launch. A font that silently fails to load is a visual regression nobody reports; now it is in the log.

### Verified (observed)
- `[m2m] startup: render webgpu, font ok` — read from the shipped binary's stdout
- `cargo test` 9 passed · clippy clean · fmt clean · tsc clean · vite build ok

### Note on review discipline
This diff (~60 lines of wiring) got a self-review rather than the full `/code-review`
agent pass: renamed `report_backend` → `report_startup` because the parameter had
grown into a composite diagnostic string and the old name no longer described it.
Full agent review is warranted for the next substantial diff (P0-10 / P1).

### Incident
A full-screen `screencapture` intended for the app window instead captured an
unrelated browser window containing the user's private messages. The file was
deleted immediately. **Do not use full-screen capture** — capture a specific
window id, or verify programmatically via the app's stdout as this session did.

### Next session starts at
**P0-10** — capture legacy solver benchmarks for all 9 templates. Must land
before any P1 solver work or the A/B comparison has no baseline.

---

## Session 001 — 2026-08-29

**Started:** 2026-08-29 15:52:11
**Ended:** 2026-08-29 16:48:38
**Focus:** Grounding, architecture decisions, memory bootstrap, port foundation

### Completed
- **P0-1** Grounding pass over the legacy codebase and toolchain
- **P0-2** All 13 `memory/` documents created
- **P0-3** Legacy app moved to `legacy/` by `git mv` — history preserved
- **P0-4** Tauri scaffold; `tauri-cli` 2.11.4 installed via npm
- **P0-5** Rust workspace: 4 crates, all `#![forbid(unsafe_code)]`
- **P0-7** CI created — none existed before this session
- **P0-8** `.cargo/config.toml`, `.gitignore`
- **P0-6** *(partial)* `sonar-scanner` 8.1.0.6389 installed, config written
- **R-1** SOTA survey → `docs/research/skinning-sota.md`

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

### Verification performed (all observed, none assumed)
- `legacy/`: **107/107 vitest tests pass** and `vite build` succeeds after the move
- `cargo test --workspace`: **9 passed**; `clippy -D warnings` clean; `cargo fmt --check` clean
- `npx tsc --noEmit` clean; `vite build` produces 8.85 kB JS
- App bundles (**6624 KB**, budget 40960 KB) and launches: **98 MB idle RSS**, **0.1% idle CPU**
- IPC round-trip confirmed live — the status bar rendered `v0.1.0 · native core ready`, sourced from the Rust `build_info` command
- Target triple `aarch64-apple-darwin` confirmed embedded in the release binary
- Dialog capability confirmed compiled: `gen/schemas/capabilities.json` → `['default']`

### Code review findings — all 9 fixed in commit 2
One was a genuine security bug, verified independently before fixing:
- **HIGH** `vite.config.ts` had `envPrefix: ['VITE_', 'TAURI_']`. Vite's `loadEnv` copies **every** matching `process.env` key into `import.meta.env` (`dep-Dm0c1Wj2.js:16967`, read directly), and `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are real Tauri variables (confirmed by `strings` on the CLI binary). A CI build with updater signing would have shipped the private key inside the app. → narrowed to `TAURI_ENV_`.
- **MED** `TAURI_DEBUG` is the v1 name; v2 uses `TAURI_ENV_DEBUG` — sourcemaps were permanently off.
- **MED** `tauri-plugin-dialog` registered with no capability → every dialog call would be ACL-rejected at runtime.
- **MED** `BuildInfo.target` returned `ARCH` ("aarch64"), not a triple as documented.
- **MED** window `minWidth` 1024 < shell's real 1120 minimum → inspector clipped, no scrollbar.
- **LOW** ×4: arch-gate grepped use-statements but a dep arrives via `Cargo.toml`; bundle budget floored 40.9 MB to a passing "40"; dead `lint`/`test` npm scripts; unmarked missing forward navigation.

**Lesson for future sessions:** the review caught things local green checks could not. `cargo test` + `clippy` + `tsc` all passed with the signing-key leak in place. Run `/code-review` before every commit, as `session_start.md` §6 requires.

### Blockers
- **P0-6** SonarQube *server* needs Docker Desktop running (daemon verified not running) or SonarCloud credentials.
- **`references/`** — the 7 Mixamo FBX files are gitignored pending a licensing decision: royalty-free to use but not CC0, and this repo licenses all art as CC0.

### Git
- Branch `port/tauri-rust-foundation`, 2 commits, pushed to `SeedeXR/mesh2motion-app`
- PR #1 opened **against the fork's own main**, deliberately not against upstream `Mesh2Motion/mesh2motion-app`
- First CI run: 5/6 green (arch-gate, frontend, legacy suite, rust test, rust lint); bundle job still running at handover

### Next session starts at
**P0-10** — capture legacy solver benchmarks for all 9 templates. This must
happen **before** any P1 solver work or the A/B comparison has no baseline.

Then **P0-9** (vendor Asta Sans), **P0-11** (verify WebGPU availability in this
WKWebView — currently assumed WebGL2 fallback, unverified), and **R-2**
(geodesic voxel binding algorithm doc) before P1-1.

### Notes
- `legacy/` is a **test dependency**, not dead code — it is the A/B baseline for P1-8. Do not delete it.
- Capture legacy benchmarks (**P0-10**) *before* touching the solver, or the A/B comparison has no baseline.
