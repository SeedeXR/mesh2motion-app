# Session Handover Log

Newest entries at the top. Every session appends one entry before exiting.
Timestamps are local (macOS, `date "+%Y-%m-%d %H:%M:%S"`).

---

## Session 014 — 2026-08-30

**Ended:** 2026-08-30 04:48:57
**Focus:** FBX mesh geometry (P2-4a)

### First: fixed an overlap in my own planning
Session 013 split P2-3 into parts A–D without checking P2-4's scope. Geometry
and deformers live in `GeometryParser.ts`, not `FBXTreeParser.ts`, so B and D
duplicated P2-4. Folded them in; P2-3 keeps only the DOM layer and the
model/bone hierarchy.

### Measured before choosing a triangulator
The legacy triangulates with earcut over a projected tangent plane, which
handles concave n-gons. Before porting that, I counted what the reference rig
actually contains: **only triangles and quads** — 172 + 14,050 on Beta_Surface,
1,400 + 9,720 on Beta_Joints, zero n-gons.

So earcut is not needed. A quad splits along the diagonal whose two triangles
both agree with the polygon's Newell normal — **exact** for a quad, not a
heuristic — and anything larger is fanned and counted in the report so an
approximation is never applied silently.

Mutation-tested: replacing the rule with a naive fan makes the concave-quad test
fail with *"triangles wound oppositely: -4 and 0.5 — the split went outside"*.

### Results on the real rig
Beta_Surface 14,232 verts → **28,272 triangles**; Beta_Joints 10,514 → **20,840**.
Both derived from the measured polygon mix rather than asserted as a bound.
Character height comes out at a plausible centimetre scale, and normals are unit
length under **both** mapping types — the two meshes happen to use different
ones (`ByPolygonVertex`/Direct and `ByVertice`/Direct), so one file covers both
resolution paths.

### The piece P2-4b will need
`vertex_source` records the original FBX vertex behind each expanded corner.
Vertices expand per polygon corner because normals and UVs are stored per
corner, so skin weights — which are indexed by *original* vertex — cannot be
bound without the mapping back. The legacy calls this `remapSkinIndices`.

### An assertion of mine that was wrong
I asserted the concave test quad had area 1.5. The parser returned 1.75, and an
independent shoelace calculation confirmed **1.75** — my arithmetic, not the
code. Worth noting because the failure looked at first like a triangulation bug.

### Next session starts at
**P2-4b** — skin clusters: per-bone vertex indices, weights, and bind
transforms, bound through `vertex_source`.

---

## Session 013 — 2026-08-30

**Ended:** 2026-08-30 04:25:20
**Focus:** P2-3 part A — the FBX DOM layer

### Scope decision, measured before deciding
`FBXTreeParser.ts` is 1621 lines, and the todo treated that as the size of the
port. Counting per method: **319 lines are textures and materials, 218 are
lights and cameras**, and most of the 181-line `parseParameters` is material
parameters — about **46%** of the method code. None of that belongs in
`m2m-io`, which exists to read geometry and rigs; the viewport loads materials
itself. The port is roughly 700 lines, not 1620. Recorded as a struck-through
scope change with the numbers.

### Landed: the DOM layer
Connection graph, objects addressable by id, `Properties70` flattened into named
typed values. This is the semantic reshaping both readers deliberately deferred
— done **once** here, where the legacy does it twice (in
`BinaryParser.parseSubNode` and again in `TextParser`).

On the real rig: 642 objects — 67 Model, 131 Deformer (129 Cluster + 2 Skin),
315 AnimationCurve, 2 Geometry, 2 Pose. `mixamorig:Hips` resolves with subclass
`LimbNode` and `Lcl_Translation` Y = **104.3 cm**, a plausible hip height in the
centimetres Mixamo exports — a physical check, not just a structural one.

### The claim from the last two sessions, now actually asserted
P2-1 and P2-2 both claimed the readers produce one shape. Session 012's review
showed that claim was false and my test could not have caught it. So this
session asserts it **directly**: the same document is built through both paths —
ASCII text, and the node tree a binary file decodes to, with the name encoded
the other way round and integers keeping their declared width — and the
resulting models are compared for object identity, relationships and property
values.

Mutation-verified both ways: breaking `split_object_name` fails it on object
identity, and swapping the connection direction fails the graph test with
`geometry ... has parents []`.

### Next session starts at
**P2-3 part B** — Geometry: vertices, indices, normals, UVs. The DOM gives it
`objects_of_kind("Geometry")` and the Model each one hangs off.

---

## Session 012 — 2026-08-30

**Ended:** 2026-08-30 03:18:43
**Focus:** P2-2, the FBX ASCII reader

### Result
Produces the **same `FbxDocument`** as the binary reader, so the DOM layer in
P2-3 never has to know which format a file arrived in. All 8 legacy test cases
ported, plus truncation and depth guards. 29 tests in `m2m-io`.

### Where the normalisation line sits
The two formats express the same tree differently: binary puts a vertex array
directly on its `Vertices` node, ASCII writes `Vertices: *9 { a: ... }` and
hangs the numbers on a child. That is a **format** quirk, so the ASCII reader
reconciles it by hoisting `a:` onto the parent.

**Semantic** reshaping — `Properties70` flattening, `Connections` collection —
stays in P2-3, matching the P2-1 decision. Each reader knows its own format's
quirks; only the DOM layer knows what nodes mean. The legacy duplicates the
semantic reshaping in both parsers; this does it once.

### The ported tests are regression guards, not coverage
The legacy `TextParser` carries three fixes over upstream three.js, each with a
test. Mutation-testing confirmed two of them still bite:

- **Un-anchoring the brace** reproduces the historical bug precisely: parsing
  the sample document then yields **only `FBXHeaderExtension`**, because a `P:`
  line containing `D:\Art\{Project}\char.fbx` reads as a block start, the
  indent drifts, and every later node is silently discarded. That is the exact
  quiet-partial-success failure mode the loop now watches for.
- **Removing the document-level property path** loses `CreationTime` and
  `Creator`, which sit outside any block; upstream dereferenced the absent node
  and crashed.

### No regex
Leading tabs are counted directly rather than building `\t{N}` patterns per
line. Simpler, no dependency, and it makes the end-of-line brace anchor an
explicit condition rather than something emergent from a pattern.

### Next session starts at
**P2-3** — `FBXTreeParser`, 1620 LOC and the largest single port. It is also
where the semantic reshaping deferred from both P2-1 and P2-2 now lives, so it
carries more than a straight translation. Expect to split it across sessions.

---

## Session 011 — 2026-08-30

**Ended:** 2026-08-30 02:51:25
**Focus:** P2-1, the FBX binary reader — first code in `m2m-io`

### Result
Parses the real 2.1 MB Mixamo export: **version 7700, 11 roots, 6099 nodes,
11.6 ms**. Structure checked against what a rigged character must contain —
67 Model, 131 Deformer, 315 AnimationCurve, 2 Geometry, 666 Connections — and
`Geometry/Vertices` decodes through the zlib path to 42,696 f64, i.e. 14,232
vertices, at a plausible centimetre scale.

### Deliberate divergence from the original
The TypeScript `BinaryParser` does two jobs in one pass: decoding the binary
container, and reshaping nodes into a convenient object (`Properties70`
flattening, `Connections` collection, single-property collapsing). The container
format is only "named nodes with properties and children"; the reshaping is
interpretation.

Split here. This layer produces a faithful typed node tree; the reshaping moves
to the DOM layer in P2-3. The original cannot be tested without its reshaping;
this can, and the tree is typed rather than `[key: string]: any`.

### No legacy tests to port
P2-1's todo entry said "with legacy tests as harness". There are none —
`BinaryParser.ts` and `BinaryReader.ts` have no test files; only FBXTreeParser,
GeometryParser and TextParser do. The harness is instead the real Mixamo file
plus hostile input.

### Trust-boundary guards, each mutation-tested
- **Depth limit.** A stack overflow aborts the process rather than unwinding, so
  it cannot be caught — this has to be a limit, not a rescue.
- **Declared-length checks before allocating**, so a file claiming four billion
  elements fails on the number rather than on the reservation.
- **512 MB inflate ceiling**, since a zlib stream can expand by orders of
  magnitude.
- **Header-only fragments rejected.** Found by the truncation test: cutting the
  file to 27–100 bytes "parsed successfully" into an empty document, because the
  footer heuristic fires immediately and the node loop never runs. A valid FBX
  always has at least FBXHeaderExtension.

Mutation-verified both guards by removing them and confirming the corresponding
test fails.

### Code review — 8 findings, 1 high, all resolved
**The high one was a silent partial success, the worst failure mode for an
animation tool.** Cutting 578 bytes from the 2.1 MB file (0.03%) returned `Ok`
with 10 roots and 6089 nodes, having dropped the whole `Takes` section — every
animation stack — because the end-of-content test is an offset heuristic and a
cut inside the last root simply stops the loop. My `roots.is_empty()` guard only
caught total loss.

Fixed by validating the 16-byte FBX footer magic, which I first verified is
identical across all 8 real Mixamo exports available (two separate export
batches). Mutation-tested: removing the check reproduces the reviewer's exact
symptom, "a truncated file parsed with 10 roots and 6089 nodes".

Also: the inflate limit was the fixed 512 MB rather than the array's declared
size, so a property declaring one element could still inflate half a gigabyte,
and the ceiling was charged per property with the result retained — ten of them
in a 5 MB file would peak around 5 GB. Now bounded by the declared size and by
the maximum deflate ratio. A mid-list null record used to `break` and then seek
past its siblings, silently producing a smaller tree than three.js would — a
parser differential; now an error. And `expect`/`unreachable!` in parser code
contradicted the crate's own no-panic rule, so both are gone.

### A test expectation of mine that was wrong
I asserted that truncating to `len - 1` must fail, then found it parsed all 6099
nodes and wrote a test asserting *that* instead — reasoning the byte was footer
padding carrying no node data. The footer check reversed it again: losing any
footer byte means completeness cannot be verified, and given the alternative is
silently dropping animation, strict is right. The test is now the regression
guard for the 578-byte case.

One more: after the null-record change, **all 600 corruption samples are
rejected** where some previously parsed. I had added `assert!(parsed > 0)` to
keep the accept path exercised; that penalises the parser for getting stricter,
so the count is reported rather than asserted.

### Next session starts at
**P2-2** — the FBX ASCII reader (`TextParser`), which *does* have legacy tests
to port as a harness.

---

## Session 010 — 2026-08-30

**Ended:** 2026-08-30 02:25:28
**Focus:** R-3 decision (closing P1-10), and P1-11 budget benchmarking

### R-3: Robust Biharmonic Skinning is OUT
Read the **full text** this time, not the abstract the earlier survey entry was
based on. Appendix A settles it: the implementation is PyTorch plus custom CUDA
kernels plus **OptiX, OWL and NVIDIA Warp** — all NVIDIA-only, on a project
targeting Apple Silicon. Reported **71.74 s** on Bunny (against BBW's 18.32 min,
so a real 15x speedup, but our budget is 3 s).

The ray tracing is not incidental: the kernel is
`k_rt(x, xi) = V(x ↔ xi) · exp(-‖x-xi‖²/2σ²)`, tracing a ray between every
candidate point pair to test visibility. That visibility term *is* the
robustness. Reimplementing it on Metal would be redoing the paper's central
contribution, not adopting it.

And the robustness it buys is what we already have: the paper's motivating
failure is tetrahedralisation on non-watertight input — it quotes Blender's
"Bone Heat Weighting: Failed to find solution" — and P1-3 measured our reference
character as *not* watertight (26 boundary edges, 61 components) solving without
incident.

**P1-10 struck through as a result.** One idea kept for P3: the paper folds
artist weight painting into the optimisation as Dirichlet boundary conditions
rather than post-processing it.

### P1-11: comfortably inside budget

| vertices | voxelise | geodesic | weights | total | peak heap |
|---|---|---|---|---|---|
| 7,399 | 30 ms | 255 ms | 3 ms | **288 ms** | **30 MB** |
| **48,670** | 37 ms | 253 ms | 18 ms | **307 ms** | **44 MB** |
| 213,754 | 57 ms | 340 ms | 76 ms | **474 ms** | **129 MB** |

Budget is 3 s / 1.5 GB at 50k. Measured 307 ms / 44 MB on the 48,670-vertex row — **10x under time,
34x under memory**. `instruction.md` §5's optimisation ordering is deliberately
not applied: there is nothing to optimise against.

### The finding that matters for the UI
**Resolution is the dominant cost, not mesh density.** Solve time barely moves
from 7k to 214k vertices, because the geodesic Dijkstra runs over the voxel
grid, sized only by resolution. Resolution is cubic — 7 ms at 64, 39 at 128,
131 at 192, 295 at 256, 1004 at 384. Doubling resolution costs ~8x. A denser
mesh is nearly free; a finer grid is not. The P3 resolution control has to say so.

### Code review — 12 findings, 2 high, all resolved
Both high ones were mine, and both were errors of the kind this loop keeps
surfacing: a claim stated more confidently than the evidence supported.

1. **The budget test never measured a ~50k mesh.** My subdivision is `V + 3T`,
   not `4V` — it welds nothing — so the sequence is 7399 → **48670** → **213754**,
   not the "~29k → ~118k" my own docstring claimed. A `>= 50_000` gate then
   rejected 48670 by 1330 vertices and asserted on a mesh **4.3× larger** than
   the budget describes. Now picks the measurement closest to 50k, and the
   docstring states the real growth factor.
2. **I wrote that the authors do not state they will release code. They do** —
   "We will release the code upon acceptance", and it has since been accepted.
   I had it exactly backwards in a decision document, on one of the three
   conditions listed for reopening the decision.

Also corrected in the R-3 doc: I compared 71.74 s against the **3 s fast-path**
budget when `test.md` §6 has a **12 s high-quality** row whose note literally
reads "biharmonic refinement" — which is what P1-10 reserved this method as. The
honest multiple is ~6×, not 10–25×. The decision still stands, but on
**platform** rather than speed. Also: the 32.2 s mesh is *Gear*, not "filigree"
(I took that from a nearby caption); Warp does run on macOS CPU-only so the
blanket "no macOS implementation" was wrong for one of four libraries; the
paper's own timing table is not uniformly like-for-like ("unable to match the
internal mesh density in three cases"); and attributing the robustness to the
visibility term was my inference, where the paper attributes it to being
mesh-free.

In the allocator: no `realloc` override meant every `Vec` growth became
alloc + copy + dealloc, inflating both the timing and the transient peak it
asserts on — the 7399-vertex peak fell 39 MB → 29.7 MB once fixed. Plus cache-line
padding on the counters, the measurement lock extended over fixture loading and
subdivision, and a discarded warm-up call so rayon pool spin-up is not in the
denominator of the resolution ratio.

### Measurement bug caught in my own harness
The tracking allocator is process-global and `cargo test` runs tests
concurrently, so two measurements in flight corrupt one counter. Not
hypothetical: the 7399-vertex peak read **66.7 MB** with both tests in parallel
against **39.3 MB** measured alone. Serialised with a mutex, then verified by
running twice and checking the peaks reproduce exactly (39.1/39.3, 44.5/44.5,
128.9/128.9).

Also mutation-tested the budget assertion by tightening it to 1 MB and
confirming it fails with the real figure.

### Next session starts at
**P2-1** — porting the FBX binary reader, the first piece of `m2m-io`. P1 is
complete.

---

## Session 009 — 2026-08-30

**Ended:** 2026-08-30 02:05:11
**Focus:** P1-8 A/B across all nine templates, and the P1-9 decision it gates

### Result: every template improves on both metrics

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

0 fallback vertices anywhere. **P1-9 follows:** the three legacy correctors are
not ported, because the defect they patch no longer exists.

### The A/B needed the root/leaf mask to be honest
The legacy solver excludes the root and leaf bones by name. Without the same
exclusion the geodesic solver would be free to weight bones the legacy one never
considered, and the comparison would flatter it. The rig fixture format now
carries a per-bone flag byte, so the mask is data rather than a naming
convention in `m2m-core`.

### Code review — 8 findings, all resolved
The high one broke the comparison: **my leaf/root flags did not match the legacy
skip set**. Legacy is `name === 'root'`, or childless **and** named `leaf`/`tip`
(`Utilities.is_leaf_bone`). I used a structural rule — "no Bone child", "no Bone
parent" — which looks equivalent and is not. It excluded 8 `wing_feather_*_2_*`
bones on the bird that are childless but not name-marked, so the bird A/B ran
39 bones against legacy's 47. Fixed to the exact predicate; bird went 39 → 47
weightable and its result *improved* (11.1% → 8.2%), so my version had been
pessimistic there rather than flattering — but it was not comparable either way.

Also: the `LEGACY` table was rounded to whole percent, moving the pass bar by up
to half a point (bird 60.637 → 61.0), now exact and with the total bone count
asserted so a fixture cannot be paired with the wrong rig; an unfalsifiable
`mean_raw >= mean` assertion removed and the raw figure actually printed, since
it is the like-for-like one the comment claimed; and `real_mesh`'s human A/B was
still running an all-true mask against the same legacy numbers — the two now
agree at 9.5% and 2.46.

### I miscounted, and caught it by filtering
First run reported unreachable bones on five templates — bird 2, horse 2,
fish 6, dragon 8, spider 8. Alarming for exactly the creature types this project
exists to serve. Filtering to **weightable** bones only: bird 0, horse 0,
dragon 0, spider 0, fish 2. The rest were leaf bones, which legitimately sit
outside the mesh because they exist only to orient their parent.

The real case is genuine: **`rig-shark.glb`'s `back_fin_2_l/r` bones sit outside
`model-shark.glb`** — indices 13 and 17, a mirrored pair at x = ±0.451,
z = −1.83. I first called these "pectoral fin" bones, inferring from the
mirrored x without checking the names; review caught it. Several fin *tip* bones
are outside too, but those are leaves the mask already excludes. Pinned by
**index**, not by count — a count would still pass if these two became reachable
while two others died.

### Mutation-tested before committing, per last session's lesson
Regressed the solver to rigid single-bone assignment and confirmed the A/B fails
with per-template diagnostics ("human: mean influences 1.00 did not beat legacy
1.13"), then confirmed it passes when restored. The guard is real, not assumed.

### Next session starts at
**P1-10** (optional biharmonic refinement, gated on R-3) or **P1-11**
(benchmark against the `test.md` §6 budgets). P1-11 is the more useful next
step — the A/B measured quality, not yet resource usage against budget.

---

## Session 008 — 2026-08-30

**Ended:** 2026-08-30 01:50:57
**Focus:** P1-7 — finish the invariant suite

### Completed
**P1-7.** All 8 invariants covered across synthetic geometry, the real 7399-vertex
character, and randomised `proptest` generators. 65 tests.

### Invariants 7 and 8 needed splitting
Bone **assignment** is exactly scale-invariant and exactly mirror-symmetric —
0 mismatches at every resolution tried. Weight **values** only converge, because
the geodesic path is a chain of discrete voxel steps.

Both errors halve per resolution doubling — first-order, which is the signature
of discretisation rather than a systematic bias. The tests assert **convergence**
rather than a fixed tolerance, which is what makes them a guard against bias
instead of against a magic number.

### proptest earned its place immediately
Found a scale-invariance failure the fixed-fixture test could not reach, and
**shrank it to a minimal case** (base 0.05, factor 26.0). That is the capability
the hand-rolled parameter sweeps elsewhere in this crate do not have.

### The review mutation-tested my work, and it was right
It reverted each production change in a scratch tree and re-ran the suite. Three
findings, all confirmed by re-measuring:

1. **My seed-distance change made mirror symmetry 2.5x WORSE.** I justified it
   as improving scale invariance and never measured its effect on symmetry.
   Zero-seeding is symmetric *by construction*; distance-seeding bakes sub-voxel
   misalignment into the field. Measured 0.0182 -> 0.0326 at res 24, and the
   same ratio at every resolution. **Reverted**, with the numbers recorded in
   the code so nobody re-tries it blind.
2. **My documented table was stale** — it held the pre-change numbers, so
   `assert!(fine < 0.01)` was passing on 0.00925 with a 7.5% margin, not the
   2.7x the comment implied.
3. **Neither production change was pinned by any test.** Reverting both left
   every one of ~500 new lines of test passing. Worse: I had loosened a
   tolerance 100x (1e-3 -> 0.1) in the same diff, and *that* is what absorbed
   the change. Loosening a tolerance and adding the code it was loosened for, in
   one commit, hides exactly this.

Also reverted the voxel dims snapping: safe, but the deterministic sweep is
bit-identical with it removed, so it was doing nothing.

### Lesson
Twice now the harness has been the thing at fault rather than the code — session
007's NaN-blind guard, and this session's tests that could not detect the
removal of what they were written to justify. **A test added alongside a change
must be checked against the change's absence.** Worth doing by hand on anything
non-trivial rather than waiting for a reviewer to mutation-test it.

### Next session starts at
**P1-8** — A/B against the legacy solver on all 9 templates, not just the human,
recording a verdict per template. That is the gate for P1-9, deleting the three
legacy weight correctors.

---

## Session 007 — 2026-08-30

**Ended:** 2026-08-30 01:17:25
**Focus:** P1-5 weight assignment + P1-6 normalisation/pruning — the pipeline now runs end to end

### Completed
**P1-5 and P1-6** together; they are one pass. 58 tests.

### The payoff, measured against the P0-10 baseline
Same model and rig the legacy baseline used:

| | legacy | geodesic |
|---|---|---|
| single-influence vertices | **87%** | **9.0%** |
| mean influences per vertex | **1.13** | **3.66** |
| vertices needing fallback | — | 0 |
| full solve | — | 292 ms |

Smooth deformation needs 2-4 influences near a joint. That the legacy solver
averages 1.13 is precisely why it needs a smoothing pass and three
per-body-part correctors; blending by geodesic falloff produces smooth
boundaries directly.

### Design decisions
- **The falloff function is ours, not the paper's.** R-2 left it unverified, so
  rather than inventing a citation: modified Shepard over the k nearest bones,
  cutoff at the surplus (k+1)-th distance so weight reaches exactly zero there
  and the blend does not step at the fourth influence.
- **Pruning is a caller-supplied boolean mask, not name inspection.**
  `m2m-core` must not carry a naming convention. The legacy invariant (root and
  leaf bones hold no weight) is preserved by whoever builds the mask, which
  will be `m2m-rig`.

### Code review — 11 findings, 4 high, all resolved
The most damning was not a code bug but a **test** bug:

- **`first_unnormalised` could not see NaN.** `(NaN - 1.0).abs() > tol` is
  `false`, so a NaN vertex read as correctly normalised — and this is the
  function every other test in the module leans on. Combined with `influences`
  filtering on `> 0.0` (also false for NaN), **a fully-NaN solve would have
  passed most of my assertions.** Both fixed, with a regression test for the
  guard itself.
- `1.0 / d` overflows to infinity below ~2.9e-39, and `powf` overflows at a
  high exponent — either gives NaN after normalisation. Fixed by expressing
  distances as **ratios to the nearest**, so every value is <= 1 and neither
  operation can overflow. That also makes invariant 7 exact rather than
  approximate. My scale test only swept *upward* (1.0 -> 10.0), so it missed
  this entirely; it now sweeps down to 1e-6.
- An all-masked rig left vertices at all-zero weights, silently breaking
  invariant 1 — and only on meshes that happen to have unreachable vertices.
  Now rejected up front.
- The zero-total branch left stale values in the other slots.
- The falloff had two branches with an arbitrary `* 2.0` cutoff. Replaced: with
  no surplus bone the cutoff is effectively infinite, which degrades to inverse
  distance weighting — the *continuous limit* of the other branch, not a
  separate rule.
- The artist-facing falloff parameter was unvalidated; 0.0 collapsed to uniform
  blending, negative produced NaN. Now clamped, with a hostile-value test.
- Tie-breaking was by descending bone index, which would break invariant 8
  (mirror symmetry) on a symmetric mesh with exact ties.
- The perf assertion was gated behind `!cfg!(debug_assertions)`, so it never
  ran under the `cargo test --workspace` that `test.md` §8 mandates. Now a
  different ceiling per profile rather than no assertion.
- Two test comments over-claimed which invariants they covered.

### Recurring mistake — third occurrence
Wrote `!(x > 0.0)` again, the same clippy pattern fixed in `mesh.rs` (session
004) and `voxel.rs` (session 005). Fixing an instance three times without
generalising is its own signal.

### Next session starts at
**P1-7** — finish the invariant suite: invariant 8 (mirror symmetry) is still
uncovered, and the existing checks are fixed fixtures rather than `proptest`
generators.

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

---

## Session 016 — 2026-08-30 — P2-4b: FBX skin clusters

**Done.** `crates/m2m-io/src/fbx/skin.rs` + 20 tests in `crates/m2m-io/tests/fbx_skin.rs`.
Workspace: 138 tests green, clippy `-D warnings` clean, fmt clean, `tsc`/`vite build`
clean, legacy vitest 107/107.

### What it does
Reads FBX skin deformers — per-bone vertex indices, weights, bind matrices — and
remaps FBX's per-original-vertex weights onto the per-corner expanded geometry from
P2-4a via `vertex_source`. Measured on the reference rig: 129 clusters over 2 skins,
26,745 index/weight pairs, 0 unweighted corners, 0 over the influence limit.

### Two things worth remembering
- **The reference Mixamo rig stores only ~1.08 influences per vertex** — 94% of
  Beta_Surface corners are single-bone (max 3; Beta_Joints maxes at 2). Confirmed two
  independent ways. Do **not** treat Mixamo's own weights as a blending quality bar
  to beat; they are barely blended. Relevant to how P3 retargeting is evaluated.
- **40 of 129 clusters have no `Indexes` node at all**, every one a finger bone. A
  bone can be bound while influencing nothing. Pinned by a test, because a change that
  started dropping them would shift every later bone index.

### Bind matrix — deliberate divergence from the legacy
Legacy uses `inverse(TransformLink)` and supplies the mesh transform separately as
three.js's `bindMatrix` from the rebuilt scene graph. This port folds them into
`inverse(TransformLink) * Transform` — same composition, but from what the exporter
recorded rather than from reconstructing the graph identically. All 129 clusters carry
a non-identity `Transform` (worst deviation 179.9), so this is not cosmetic.

### Process note — the mutation discipline earned its keep this session
23 mutations run. **Four survived on the first attempt, and none of them was a
redundant test:**
- Two guards were redundant *for the input I picked*, so either could be deleted with
  tests green. Needed a case that discriminates them.
- One assertion read through `influences()`, which filters zero-weight slots — so it
  literally could not observe the defect it claimed to cover. Had to assert on raw slots.
- One branch (non-finite matrix) had no coverage at all; the determinant check cannot
  catch it because `NaN == 0.0` is false.

I also wrote a test whose comment overclaimed what it pinned, and shipped a review
brief asserting "each SkinReport field has a test that fails without it" — two fields
had none. **Verify the claim before making it, including about my own tests.**

### Review findings: 9, all real, all fixed
Every one was a quiet partial success — `Ok` returned with deformation wrong or
missing. Detail in `memory/todo.md` under P2-4b. The most dangerous was an index/weight
desync that a corrupt file triggers trivially and that reported itself as a benign
ragged array. I found that one independently while reviewing my own diff; the
subagent confirmed it with the same conclusion.

`unweighted_vertices` was **deleted rather than fixed** — it counted source vertices
while `fallback_vertices` listed corners, so the two disagreed ~6x for the same event.

### Wrong turn worth not repeating
I ran `npx vitest run` from the repo root and reported 7 legacy failures. CI runs it
with `working-directory: legacy`, against legacy's own `node_modules`. Run it the way
CI does before concluding anything is broken: `cd legacy && npx vitest run`.

### Next
P2-3 part B (Models and the bone hierarchy) — the last piece of `FBXTreeParser` — then
P2-5 `AnimationParser`. `references/` licensing is still an open question for the user.

---

## Session 017 — 2026-08-30 — P2-3B: Models and the bone hierarchy

**Done.** `crates/m2m-io/src/fbx/transform.rs` (FBX transform pipeline) and
`model.rs` (Models, hierarchy, world matrices), 14 tests across
`tests/fbx_transform.rs` and `tests/fbx_model.rs`. Workspace: 152 tests green,
clippy `-D warnings` clean, fmt clean, legacy vitest 107/107, tsc + vite build clean.

### The method that made this work: differential fixtures
Two new dumpers run the **legacy's own code** headless and record its output:
- `legacy/bench/dump-transform-fixtures.ts` — 49 `generateTransform` cases.
- `legacy/bench/dump-model-fixtures.ts` — the whole rig hierarchy via `FBXLoader`.

`FBXLoader.parse()` is synchronous and pure JS, so it runs under the bench vitest
config with no WebGL. **Use this for the animation port too** — it is the only way
to catch convention errors that produce plausible-looking wrong output.

Worst deviation: 8.5e-14 local, 2.0e-13 world on a ~165-unit rig. f64 rounding.

### Two conventions that are invisible without a differential test
- three.js Euler order strings are the **reverse** of FBX extrinsic integers:
  FBX order 0 → three.js `'ZYX'`.
- three.js composes `'XYZ'` as the literal product `Rx·Ry·Rz` (read from
  `three.core.js`, not assumed).

Invert either and the rig still rotates smoothly, still inverts, and is wrong.

### Corpus measurement drove the scope (8 files, 522 Models)
520 `LimbNode`, 2 `Mesh`. `Lcl_Translation` 520, `PreRotation` 440,
`Lcl_Rotation` 274, `Lcl_Scaling` 63 — **all 1.0**. `InheritType` 67 (values 1
and 2 only). **Zero files carry `PostRotation`, pivots, offsets or `RotationOrder`.**

Because every scale is 1.0, **all three `InheritType` modes agree on this rig** —
which is why four mutations survived and needed synthetic non-uniformly-scaled
parents to kill. Measure before assuming a real file exercises a branch.

### A trap worth remembering about the legacy graph
`buildSkeleton` makes a **separate `Bone` per skeleton** for a Model shared
between skins, and nests the second inside the first. The rig has 132 nodes for
67 Model ids. Only the outer Bone carries `userData.transformData`. My first
dumper took "first node with this id" and would have exported an identity local
matrix for every shared bone — the hips wrong by the full 104.27 cm. The
duplicate-consistency assertion I put in the dumper is what caught it; keep that
habit when flattening a three.js graph.

### Mutations: 23 run
All killed except two proven no-ops — adjacent pure-translation matrices commute
(`T(a)·T(b) = T(a+b)`), verified numerically as exactly 0 difference rather than
argued. Four others survived the first pass and each exposed a real gap, not a
redundant test (see the InheritType note above).

Also: my own vacuity guard caught me asserting "≥5 models branch" without
measuring — it is 4 (Hips, Spine2, and each hand with 5 fingers).

### Known gaps, deliberate, recorded in todo.md as P2-3 B-follow-up
- **`BindPose` rest-pose override** for bones with no skin cluster. Measured: 0
  of 65 bones qualify on this rig, so it is a no-op here. Needs cluster
  knowledge; belongs with the rig layer.
- **`UpAxis == 2` Z-up rotation.** Corpus is all `UpAxis == 1`. Belongs at the
  importer level.

Neither is dropped silently — both are written down with their measurements.

### Review: 4 findings, 3 real and fixed, 1 stale
- **`Root` subclass was not a bone.** Legacy: `case 'LimbNode': case 'Root'`. The
  corpus is all `LimbNode`, so nothing measured could see it; a Max Biped export
  would have lost its root joint silently.
- **`InheritType::from_fbx` fallback was wrong.** Legacy sends everything that is
  not 0 or 1 into `Rrs`; this sent it to `RrSs`.
- **Shear — deliberate divergence, now documented.** The legacy round-trips every
  local matrix through position/quaternion/scale, which cannot represent shear
  (measured: a sheared matrix changes by 0.39 through three.js r185's
  `applyMatrix4` + `updateWorldMatrix`). FBX genuinely produces shear under a
  non-uniformly scaled ancestor. This port keeps it. **The fixture test only
  agrees to 8.5e-14 because every scale in the rig is 1.0** — regenerate from a
  scaled rig and it will fail, correctly.
- The `ancestors` step-limit finding was **stale**: the guard landed before the
  review reported. Always re-check a finding against the current file.

### Next
P2-5 `AnimationParser` (783 LOC). The fixture-dumper approach applies directly:
dump the legacy's parsed `AnimationClip` tracks and diff. `model.rs` keeps each
node's raw `TransformData`, which the animation layer needs as the base to apply
curves onto.

---

## Session 018 — 2026-08-30 — P2-5: AnimationParser

**Done.** `crates/m2m-io/src/fbx/animation.rs` + 14 tests. Workspace 170 tests
green, clippy `-D warnings` clean, fmt clean.

**Diffed key-for-key against the legacy: 2 clips, 53 tracks, 7,844 keys, worst
deviation exactly 0 on times and 1.7e-18 on values.**

### What the differential fixture caught that nothing else would have
Clip order is ascending **stack id**, not alphabetical — the legacy iterates a
JS object keyed by numeric id strings, which JavaScript enumerates numerically.
I had sorted by name and it failed on the first run. This is the third session
where the fixture caught an ordering/convention error invisible to inspection.

### Measure before assuming a branch is exercised (again)
Across 8 files: no `PostRotation`, no scale or morph channel, no curve node
missing an axis, **zero rotation steps ≥180°**, **zero sign flips in 7,644
adjacent key pairs**, zero non-default Euler orders, and all 53 tracks share one
end time. Five mutations survived the real-rig test purely because of this;
every one was a genuine gap, none was a redundant test.

### The mistake worth remembering
My first unroll test was **vacuous through a wrong premise**. I picked
(0,0,0)→(170,170,170) by reasoning from the XYZ half-angle formula
`w = c1c2c3 - s1s2s3`. The default order is **ZYX**, where that triple gives
w ≈ +0.99 — it can never flip. A scan then showed **no** triple with every axis
under 180° flips against the identity at all. The real case needs a non-identity
previous key: (-180,-150,-180)→(-30,-30,-30), dot -0.90.

Lesson: when a test needs a specific numerical condition, **construct it by
search and assert the premise**, don't derive it from a formula you remember.

### Trust boundary
A curve whose `KeyTime` and `KeyValueFloat` differ in length indexed past the
end of the value array while sampling — a panic reached straight from file
bytes. Truncated and counted now, matching `skin.rs`'s ragged-array handling.

### Deliberate divergences (documented in the module, not silent)
- **Morph channels are counted, not parsed.** They need the geometry's
  blend-shape list, which nothing in this project produces. A file whose facial
  animation vanished must not load looking complete.
- **The ≥180° sub-division slerps in quaternion space** rather than
  round-tripping through Euler as the legacy does (lossy at gimbal lock), and
  emits the interval endpoint the legacy's `for (t=0; t<1; …)` loop drops.
  Measured unreachable on every file in the corpus.

### Review: 5 findings, all real, all fixed
Two were trust-boundary defects reachable from file bytes: an **unbounded
subdivision count** (an infinite span saturates the float→int cast to
`usize::MAX`, so the loop never ends; 1e7 degrees alone asks for 55,557 keys),
and **non-finite keys becoming NaN quaternions** with nothing in the report.

The one worth remembering: **I wrote a comment asserting `synchronise`'s
shortcut could not change output, and it was wrong.** It compared key *counts*;
a curve with a duplicated `KeyTime` has more values than distinct times, so it
matches the deduplicated merged length while its keys sit elsewhere. The
reviewer produced the counterexample. Comparing the times themselves fixes it —
**the legacy still has this bug**. Lesson: a claim that a branch is
output-neutral is a proof obligation, not a comment.

Also: every layer was applied where the legacy takes only the first (a stack
with an override layer would emit two tracks for one bone), and my
clip-ordering comment overclaimed how JavaScript enumerates numeric keys
(numeric order only below 2^32).

### Next
P2-6 is the **FBX writer** — net-new, no Rust prior art, and the first piece
with no legacy implementation to diff against. That changes the method: the
gate becomes round-tripping our own reader over what we write, plus loading the
result in the legacy loader (which runs headless) and in Blender via MCP.
P2-7 (glTF via the `gltf` crate) may be the better next step since it has a
spec and a crate; consider reordering.

---

## Session 019 — 2026-08-30 — P2-8 fuzzing + P2-9 hostile corpus

**Done.** `crates/m2m-io/fuzz/` (three targets, seeds, `seed.sh`), a `fuzz` CI
job on PRs and nightly, and `crates/m2m-io/tests/fbx_hostile.rs`. Workspace 189
tests green in release; m2m-io 121 green in **debug**, which is the mode that
matters this session.

### Reordering P2-8 ahead of the writer paid for itself twice
**Two real trust-boundary bugs, both found within minutes:**

1. **`debug_assert!` on file content** (`Scene::from_document`). An `Objects`
   child with no numeric id panicked *every debug build*, and CI's
   `cargo test --workspace` **is** a debug build — it had simply never been
   given that input. A second `debug_assert!` on duplicate ids was the same
   mistake. Both are `SceneReport` counters now.
   **An assertion is for an invariant this code controls. What a file contains
   is never one.** Worth grepping for `debug_assert` in any new parser code.
2. **A non-finite `Lcl_Translation` turned an entire subtree to NaN**, because
   a child multiplies by its parent's world matrix. Found by the §4 corpus, not
   the fuzzer. Components are validated at read time and replaced individually
   so the node's other valid components survive.

### Two things I would have got wrong without measuring
- **Deep ASCII does not test the depth cap.** The text reader uses an explicit
  stack; only the binary reader recurses. The test now builds a nested *binary*
  file, and pins the exact boundary: outermost node is depth 0, so 257 levels
  pass and 258 fail.
- **`fbx_pipeline` is the target that finds things.** `fbx_binary` and
  `fbx_text` ran 190k and 8M iterations with nothing. Both hand-found bugs from
  017/018 and both bugs found here were in LAYERS above the readers.

### Method note for the next parser
Binary FBX starts with a 23-byte magic that random mutation essentially never
reproduces, so the pipeline target reaches the layers mainly through the ASCII
seeds. If GLB fuzzing (P2-7) matters, seed it with real GLBs and consider a
structure-aware generator — otherwise the fuzzer spends its whole budget
failing the header check.

### Toolchain now installed
`rustup toolchain install nightly --profile minimal` and `cargo install
cargo-fuzz` (0.13.2). Run with `cargo +nightly fuzz run <target>`.

### Disk — the memory was stale and I corrected it
`session_start.md` claimed ~34 GB free since session 001. It is **~15 GB** on a
460 GB disk that is 97% full. `fuzz/target` adds ~220 MB and `fuzz/corpus`
grows past 300 MB; both are gitignored and safe to delete.

### Review: 12 findings, and it was worth the hour it took
The reviewer measured things I had asserted. Two were **HIGH and real**, both
"never hang / never OOM" violations — the very contract this session was about:

1. **Quadratic key matching with no cap.** `vector_track` searched each curve's
   times per merged time. The key count comes from the file (a `KeyTime` array
   is bounded only by the 512 MB inflate ceiling = 64M keys), so ~20 KB could
   buy ~10^12 comparisons. Fixed twice over: a forward cursor per axis makes it
   linear, and `MAX_KEYS_PER_CURVE` bounds the count. **The differential test
   still matches the legacy to exactly 0 on times**, which is the evidence the
   cursor is equivalent.
2. **Unbounded geometry amplification.** A `PolygonVertexIndex` of N closed as
   one polygon fans to 3(N-2) expanded vertices; ~0.5 MB of deflate could ask
   for >10 GB. Now `MAX_CORNERS` and `MAX_POLYGON_CORNERS`.

Also real: the fuzz target hardcoded an identity `GeometricTransform`, so the
file-driven path was never fuzzed at all; `find(|r| r.name == "Objects")`
silently dropped a second `Objects` root while the new counters read zero; and
the CI loop under `bash -e` meant one crash stopped the other targets running.

**Three of my own comments were factually wrong** and the reviewer caught each:
a claim that `reader.rs` does format sniffing (it does not — it is only a
`Cursor`), a claim that a bare `1800` in a GitHub expression could evaluate as
falsy (it cannot), and a `seed.sh` warning promising the binary corpus would
fall back to ASCII seeds (nothing copies them there).

**Corpus quality was the most useful measurement:** 70.7% of the reference
rig's bytes sit inside zlib streams, where any mutation fails inflate and
rejects the whole file — so most of the 8.4M runs never reached the DOM. Added
`ascii-skinned-quad.fbx` (a quad, both layer-element mappings, a skin cluster)
with a test asserting it reaches those paths, plus `-max_len` so libFuzzer does
not adopt the 2.1 MB rig as its input size.

Deferred with reason, recorded as **P2-8b**: the per-file inflate budget.

### Next
P2-6, the FBX writer. First piece with no legacy implementation to diff
against, so the method changes: round-trip our own reader, load the result in
the legacy loader headless, and open it in Blender via MCP. P2-7 (glTF via the
`gltf` crate) is an alternative next step with a spec and a crate behind it.

---

## Session 020 — 2026-08-30 — P2-8b: per-file inflate budget

**Done.** `InflateBudget` threaded through `binary::parse`, charged with the
declared size **before** inflating. Ceilings are now 256 MB per property and
512 MB per file. Workspace 196 tests green, clippy and fmt clean, legacy 107/107.

### Why 512 MB and not the 1 GB I first wrote
This tool's stated priority is low memory use, and the reference rig inflates
1.5 MB in total. 512 MB is two orders of magnitude of headroom and still inside
what a desktop app survives. Per-property dropped 512 -> 256 MB so the two
limits stay independently meaningful rather than one subsuming the other.

### The ordering is now pinned, and it took a second attempt
My first mutation "charge after inflate" actually *deleted* the charge, so it
duplicated an earlier mutation and pinned nothing — I said so rather than
counting it. The ordering **is** observable: give the last array a declared
size but garbage instead of a zlib stream, and charging first yields
`ImplausibleLength` while inflating first yields `Inflate`. Different errors,
cheap test, and the real mutation now fails.

### CORRECTION: P2-6 does have a reference implementation
Sessions 018 and 019 both recorded "the FBX writer has no legacy implementation
to diff against". **That is wrong.** The legacy exports FBX via
**`@comfyorg/fbx-exporter-three` v1.0.1** — a third-party npm package, not
three.js core, which ships no `FBXExporter`. Verified this session: it runs
headless under the bench config exactly like the loader, and writes **binary**
FBX (`Kaydara FBX Binary`, version 7400, 2.97 MB from the reference rig).
So the differential-fixture method applies to P2-6 after all.

**And our reader parses its output cleanly** — a second, independent producer,
every report field zero across DOM/models/skins/animation. Good evidence the
reader is not overfit to Mixamo's writer. Details and the shape differences
(132 Models vs 67, 159 curves vs 315, unshared vertices, the empty stack
dropped) are recorded under P2-6 in todo.md.

### Format decision
**P2-6 before P2-7.** The app exports both (`ExportFormat { GLB, FBX }`), so
neither is optional, but P2-6 now has a runnable reference and a complete
reader to round-trip against.

### Fuzzing note
`-max_len=65536` is worth more than it looks: `fbx_binary` went from 190k runs
in 120 s to **1.73M runs in 46 s**, because without it libFuzzer adopts the
2.1 MB rig as its input size. The review predicted this; the measurement
confirms it.

### Review: 7 findings, all real, all fixed
- **The per-file rejection reused `ImplausibleLength`**, whose message reads
  "declared length N exceeds the M bytes remaining" — but nothing is truncated
  and M is not bytes remaining. Anyone hitting the cap on a legitimately large
  file would chase a truncation that does not exist. Now
  `FbxError::InflateBudgetExceeded { total, limit }`.
- **THREE stale doc comments** citing the old 512 MB ceiling — in `binary.rs`
  (the new constant's own rationale), `geometry.rs` and `animation.rs` — all
  invalidated by this very change. This is a recurring failure mode across
  018/019/020. **When a constant changes, grep for every numeric claim that
  cites it.**
- A test compressed 300 MB that was never inflated, because the per-property
  check fires on the declared count alone.
- The suite cost **13.6 s and 1.69 GB peak**; now **0.9 s and 1.33 GB**, by
  sharing fixtures through a `OnceLock` and using the smallest arrays that
  still exceed the real ceiling (three of 171 MB). The remaining peak is
  inherent to testing real 512 MB constants with real data.
- The all-zero fixture cleared the 1032:1 deflate-ratio guard by **0.24%**, so
  a `miniz_oxide` release compressing zeros marginally better would have made
  these tests fail *blaming the budget* for a compressor change. The buffer is
  salted now, ~500:1.

The reviewer also confirmed what I had reasoned about the uncompressed paths,
and found one uncharged amplifier I had missed: `Cursor::read_string` expands
invalid UTF-8 to 3-byte U+FFFD, so an `S` property of L bytes retains 3L. That
is 3x against deflate's 1032x and still bounded by file size — noted, not fixed.

### Started P2-6(a): the binary encoder
`crates/m2m-io/src/fbx/encode.rs`, the inverse of `binary::parse`, gated by
`parse(encode(parse(bytes))) == parse(bytes)`. 133 m2m-io tests green.

**Two findings from the round trip itself, both worth carrying forward:**

1. **A round trip is necessary but not sufficient.** Four of eight mutations
   survive it — the null record ending a child list, the top-level null record,
   an uncompressed array's byte length, and the footer padding. Every one is a
   conformance detail *our reader does not check*, so nothing would notice if
   the encoder stopped writing them. Only a different reader can close those.
2. **`FbxDocument` is a lossy view of object names.** Binary FBX stores
   `Name\0\x01Class` and `read_string` truncates at the NUL, so a writer built
   on the document emits names with no class suffix. Both parses truncate
   identically, so the round trip passes; the loss is in the bytes. three.js
   truncates too — Blender and Maya are unverified.

### Next
P2-6(b): builders, and the legacy-loader acceptance gate, using the differential
method against
`@comfyorg/fbx-exporter-three`. Split it as (a) a binary **encoder**, the
inverse of `binary::parse`, testable by round-tripping our own reader with no
reference needed, then (b) **builders** from scene data, diffable against the
reference. Note the reader validates a 16-byte `FOOTER_MAGIC` and `at_footer`
treats "within ~176 bytes of the end" as the footer — a writer must satisfy
both, and a file too small is unparseable for that reason (this bit me while
writing a test fixture this session).
