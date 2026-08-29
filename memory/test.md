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

Not currently installed (verified 2026-08-29). Install as `todo.md` P0-6.

```bash
brew install sonar-scanner          # scanner
docker compose -f docker/sonarqube.yml up -d   # local server
sonar-scanner -Dsonar.projectKey=mesh2motion-app
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
