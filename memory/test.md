# Testing & Verification Contract

**An implementation is incomplete until every relevant test passes and performance
sits inside the thresholds below.** This file exists to make "it works" a
measurable claim rather than an opinion.

## 1. Why this is strict

Rigging bugs are **silent and visual**. A weight solver that produces a subtly wrong
elbow does not throw, does not fail a typecheck, and does not fail a naive unit
test — it ships, and an artist discovers it three hours into animating. The only
defence is comparing real output against a known baseline on real meshes.

## 2. Required test types

| Type | When required | Where |
|---|---|---|
| **Unit** | every pure function with a branch | in-file `#[cfg(test)]` / `*.test.ts` |
| **Integration** | anything crossing a module or IPC boundary | `crates/*/tests/`, `app/tests/` |
| **Regression** | every bug fix — must fail before the fix | tagged `regression_<issue>` |
| **Property** | every numeric/geometric invariant | `proptest` |
| **Golden / visual** | solver output on template meshes | `tests/golden/` + Blender render diff |
| **Smoke** | app launches, loads a model, binds, exports | `app/tests/smoke/` |
| **E2E** | each full ProcessStep user path | WebDriver over the Tauri build |
| **Performance** | every hot-path change | `bench/` criterion |
| **Fuzz** | every parser | `cargo-fuzz` targets in `m2m-io` |

## 3. Invariants that must always hold (property tests)

For **any** mesh and **any** template skeleton:

1. Every vertex's weights sum to 1.0 ± 1e-5
2. No weight is NaN, infinite, or negative
3. At most 4 non-zero weights per vertex (GPU skinning limit)
4. Root bone and leaf/orientation bones receive exactly 0 weight
5. Every bone index is within `[0, bone_count)`
6. Solving the same input twice yields bit-identical output (determinism)
7. Solving is invariant to uniform scale of the input mesh
8. Mirrored geometry with a mirrored skeleton yields mirrored weights (±1e-4)

Invariant 6 matters more than it looks: `rayon` reductions over floats are
order-dependent. Non-determinism here makes every golden test flaky.

## 4. Hostile input corpus

Parsers are trust boundaries. `legacy/static/test-files/` is the existing corpus and
must keep passing:

- `human-interleaved-buffer-mesh.glb` — interleaved buffers
- `fox-model-gltf-missing-texture.zip`, `fox-model-missing-gltf.zip` — missing resources
- `m2m-custom-animation-wrong-bone-count.glb`, `m2m-wrong-bone-names.glb`, `m2m-valid-but-no-animation-data.glb`
- `mixamo-original-rig.fbx` — real Mixamo FBX
- `bone-correction-tests/human-a-pose.glb` — **A-pose human.** Must solve as cleanly as the T-pose default (objective O8) and is the regression guard for arm-to-ribcage weight bleed. A T-pose clip retargeted onto this bind must not drift the arms.
- `references/human_based_fbx_mixamo_animations/*.fbx` — 7 clips, round-trip corpus

Additionally required: truncated files, zero-byte files, wrong magic bytes,
declared-length-exceeds-file, deeply nested nodes, NaN/Inf floats, degenerate and
non-watertight meshes, meshes with zero vertices, and duplicate bone names.

**A malformed file must return an error. Never panic, never hang, never OOM.**

## 5. Fuzzing

**Installed 2026-08-30 (session 019):** `rustup toolchain install nightly --profile minimal`
plus `cargo install cargo-fuzz` (0.13.2). The targets live in `crates/m2m-io/fuzz/`, which
declares its own `[workspace]` so the nightly requirement never reaches a stable build.
Run them with `cargo +nightly fuzz run <target>`; assemble the corpus with `fuzz/seed.sh`.

`cargo-fuzz` targets for the FBX binary parser, FBX ASCII parser, and GLB reader.
Each runs 60 s in CI on PRs and 30 min nightly. Any crash or OOM found becomes a
regression test with the minimised input committed.

## 6. Resource budgets — enforced, not aspirational

Measured on the reference machine (**Apple M4, 10 cores, 16 GB, macOS 26.6.2**).
Every benchmark records machine, input, and run count.

| Scenario | Wall time | Peak RSS | Notes |
|---|---|---|---|
| App idle | — | ≤ 250 MB | 0% CPU — no render loop when nothing moves |
| Load 50k-vert GLB | ≤ 400 ms | ≤ 600 MB | |
| Bind skin, 50k verts, fast | ≤ 3 s | ≤ 1.5 GB | vs. legacy baseline |
| Bind skin, 50k verts, high | ≤ 12 s | ≤ 2.5 GB | biharmonic refinement |
| FBX import, Mixamo clip | ≤ 500 ms | ≤ 400 MB | |
| Export GLB, 50k + 20 clips | ≤ 2 s | ≤ 800 MB | |
| Binary size | — | ≤ 40 MB | `cargo tauri build` |

Profiling is mandatory for CPU, GPU, memory, and disk on any change to a hot path.
Use `cargo instruments` / Instruments.app on macOS; record the trace path in
`handover_session.md`.

## 7. SonarQube

**UNBLOCKED 2026-09-01** (session 050, at the user's direction to get a token via
docker). Server is `m2m-sonarqube` (`docker/sonarqube.yml`), SonarQube 26.8.0
Community on http://localhost:9000, status UP.

```bash
set -a; . ./.sonar-token; set +a      # SONAR_TOKEN, SONAR_HOST_URL, admin creds
sonar-scanner                          # reads sonar-project.properties
```

**Secrets live only in `./.sonar-token` (gitignored — never commit).** It holds
the analysis token, host URL, and the admin login.

### How the block was cleared
The admin password had been lost, and SonarQube's embedded H2 uses a password
generated per-install (so `sonar/sonar` and `admin/admin` both fail at the H2
level — confirmed via the trace file, and H2's `Recover` tool showed the DB was
only a default install plus one project registration, no analysis history). The
fix was **reversible**: the old `sonar.mv.db` and `es9/` were renamed to
`*.bak.<ts>` inside the `docker_sonarqube_data` volume, SonarQube reinitialised
a fresh DB on restart (back to `admin/admin`), the password was changed, and a
`GLOBAL_ANALYSIS_TOKEN` was minted and verified. To roll back: stop the
container, restore the `.bak.<ts>` files, restart.

### What the first scan found (HEAD 5b10d46, gate OK)
- **0 bugs, 0 vulnerabilities, 0 security hotspots.** 28 code smells, 23 of them
  CRITICAL — all `rust:S3776` cognitive complexity, threshold-of-one-over in the
  FBX/GLB parsers (e.g. 16 vs 15). Non-blocking; a refactor task, not a gate
  failure.
- **Correction to a long-standing assumption**: SonarQube 26.8 Community **does**
  ship a Rust analyser (`sonar-rust-plugin`) and analyses the crates on a plain
  scan — the earlier "no Rust analyser" note (and `sonar-project.properties`
  comment, now fixed) was true of older Community but not this version. clippy
  `-D warnings` stays the CI gate; Sonar is advisory locally.
```

Quality gate: no new bugs, no new vulnerabilities, no new code smells above Minor,
coverage on new code ≥ 80%, duplication on new code ≤ 3%.

## 8. Incremental validation

Run after **every** change, not at the end of a session:

```bash
cargo test --workspace          # < 30 s target
cargo clippy --workspace -- -D warnings
npm test
```

Full suite (golden, E2E, bench, fuzz) runs before commit and in CI.

## 9. Baseline comparison against legacy

The single most important test in this project. `legacy/` stays runnable for exactly
this reason.

```
for each of the 9 template rigs × its demo mesh:
    legacy solve  → weights_old
    new solve     → weights_new
    report: max Δ, mean Δ, vertices whose dominant bone changed
    render both in Blender headless at 6 poses → perceptual diff
```

A dominant-bone change is **not automatically a regression** — the new solver is
expected to differ, and to be better. It requires a human look and a recorded
verdict in `handover_session.md`. Silent acceptance is forbidden; so is silent
rejection.

## 10. Definition of done

- [ ] Unit + integration tests written and passing
- [ ] Regression test added if this was a bug fix (and it failed before)
- [ ] Property invariants (§3) hold
- [ ] Hostile corpus (§4) still passes
- [ ] Benchmarks within §6 budgets, no >10% regression vs. `bench/baselines/`
- [ ] `/code-review` findings addressed or deferred with a written reason
- [ ] `/ponytail:ponytail-review` — no over-engineering
- [ ] SonarQube gate green
- [ ] Baseline comparison (§9) recorded if the solver changed
- [ ] CI observed green — not assumed

## 6. Fuzzing a format you did not write the parser for

*Added session 025, after the glTF reader.*

The `glb` fuzz target found four defects in five minutes. **Three were in the
`gltf` crate, not in our code.** That is the finding worth keeping: adding a
well-maintained parser as a dependency moves the trust boundary, it does not
remove it. Everything reachable from file bytes is still our problem, because
our process is the one that dies.

What they were, and what each teaches:

| Where | Trigger | Fires in |
|---|---|---|
| ours | index accessor names a vertex that does not exist | any build |
| `gltf-json/src/mesh.rs:151` | `root.accessors[i]` inside the validation hook, before validating `i` | **release** |
| `gltf/src/binary.rs:252` | `header.length as usize - 12` underflows below 12 | debug |
| `gltf/src/accessor/util.rs:371` | `debug_assert_eq!` on the accessor's declared size; `stride * (count - 1)` at `count == 0` | debug |

- **`cargo test` is a debug build.** A `debug_assert!` on file content is a CI
  failure waiting for the right input, not a harmless annotation. Session 019
  learned this about our own `debug_assert!`s in `Scene::from_document`; it
  applies identically to a dependency's.
- **A validator that dereferences before it validates is not a validator.** The
  release panic is the serious one: it turns "open a malformed model" into
  "the app exits". Guard indices *before* handing bytes to the library.
- **Fuzz the layers, not just the entry point.** The target drives the whole
  read and then asserts the invariants callers actually rely on — every triangle
  index is a real vertex, every joint is a real node. The first crash was one of
  those assertions, not a panic: the reader returned a document that was wrong
  rather than crashing. A target that only checked "did not panic" would have
  passed it.
- **Validation must reject nothing legitimate.** After adding three guards,
  every one of the 55 real `.glb` files in the repo still reads with an all-zero
  report. That check is as important as the fuzzing: a guard that is too strict
  fails closed on real user files, which is its own bug. Gating `NORMAL` on
  VEC3/f32 was exactly that mistake — glTF allows normalized byte and short
  there, and the reader does not read normals at all.

### Comparing against an independent reader across formats

`tools/blender-fbx-import-check.py` now imports `.glb` as well as `.fbx`, so one
report shape covers both, and `tools/glb-blender-diff.sh` sweeps the corpus.
Two things made the comparison lie before it told the truth:

1. **Blender's glTF importer fabricates geometry.** It adds an icosphere as a
   bone display widget, which appears in `bpy.data.objects` as a real MESH —
   a phantom 42-vertex, 80-polygon mesh on every skinned file, including one
   whose JSON declares no meshes at all. `disable_bone_shape=True`.
2. **Count the same thing on both sides.** A glTF mesh holds one primitive per
   material and importers merge them, so primitives are not mesh objects
   (`human-jay.glb`: 22 primitives, 1 mesh). Four files looked wrong until the
   comparison was fixed, and the reader was right the whole time.

Both cost real time, and both would have been read as "our reader is broken".
When an independent reader disagrees, **find out what it is actually counting
before changing anything.**
