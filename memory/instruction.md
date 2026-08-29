# Implementation Guide

## 1. Directory structure

```
mesh2motion-app/
├── memory/                  agent memory (this folder) — read session_start.md first
├── legacy/                  original TS/Three.js app. RUNNABLE. Do not delete.
├── crates/
│   ├── m2m-core/            pure geometry + skinning solver
│   ├── m2m-io/              FBX / GLTF / GLB
│   ├── m2m-rig/             templates, fitting, retarget, automap
│   └── m2m-bridge/          Blender DCC bridge
├── src-tauri/               Tauri shell — thin command layer only
├── app/                     frontend (TypeScript + Three.js)
│   └── src/{ipc,viewport,steps,ui,state}/
├── assets/                  vendored fonts, icons, template rigs
├── references/              Mixamo FBX corpus (read-only inputs)
├── docs/                    generated + authored documentation
├── bench/                   criterion benchmarks + baselines
└── .github/workflows/       CI
```

## 2. Coding standards

### Rust
- Edition 2021+. `#![forbid(unsafe_code)]` in every crate unless a documented, benchmarked exception exists.
- `cargo clippy --workspace -- -D warnings` must be clean. No `#[allow]` without a comment naming the reason.
- `rustfmt` default config, no overrides.
- **No `unwrap()` / `expect()` outside tests and `main`.** Parsers return `Result`. A malformed FBX errors; it never panics.
- Public functions in `m2m-core` take and return plain data (`&[f32]`, `Vec<u32>`, `glam` types). No trait-object indirection without two real implementations.
- Errors: `thiserror` for library crates, `anyhow` only at the `src-tauri` boundary.

### TypeScript
- `strict: true`. No `any` — use `unknown` and narrow.
- ESLint config inherited from `legacy/eslint.config.js`; keep it passing.
- `invoke` is called **only** from `app/src/ipc/`. Everything else imports typed wrappers.
- Keep the legacy `snake_case` method naming where porting a file 1:1 — matching the reference implementation matters more than style purity during the port.

### Both
- Comment density matches surrounding code. Comments say *why*, never *what*.
- Deliberate shortcuts get `// ponytail: <ceiling>, <upgrade path>`.

## 3. Module conventions

- One concept per file. A file over ~500 LOC needs a reason.
- `mod.rs` re-exports the public surface; internals stay private.
- Every `m2m-core` module ships with `#[cfg(test)] mod tests` in-file for units, plus an integration test in `tests/` for anything crossing a module boundary.
- No module in `m2m-core` may `use tauri::` or perform I/O. This is enforced in CI by a grep gate.

## 4. Process: how a task gets done

1. **Read** — the legacy implementation, the callers, the tests. `porting.md` §7 has the mapping.
2. **Ladder** — `philosophy.md`. Stop at the first rung that holds.
3. **Test first for ported logic** — port the legacy test file, watch it fail, then port the implementation.
4. **Implement** — smallest diff that works.
5. **Verify** — run it on real input from `legacy/static/test-files/`, not a synthetic fixture.
6. **Benchmark** if it is in a hot path; compare against `bench/baselines/`.
7. **Review** — `/code-review` then `/ponytail:ponytail-review`.
8. **Record** — update `todo.md` and `handover_session.md`.

## 5. Optimisation principles

Ordered by real payoff on Apple Silicon. Do not skip ahead.

1. Remove copies (unified memory — a copy is pure waste)
2. Remove allocations from hot loops
3. `rayon` across the data before SIMD inside the kernel
4. Cache-friendly layout — SoA over AoS for vertex data
5. Only then: `Accelerate` / Metal compute

**No optimisation lands without a before/after `criterion` run on the M4 with the
input named.** A number without provenance is not a benchmark.

## 6. Adding a creature template

The whole point of the project. Keep this cheap:

1. Add the rig `.glb` to `assets/rigs/`
2. Add a template definition (data, not code) declaring bone chains, symmetry axis, and landmark hints
3. Add fitting landmarks — the points the auto-fitter aligns to the mesh
4. Add a fixture to the solver regression suite
5. Write the in-app guidance copy for that creature

**If step 2 requires new Rust code, the template system is wrong — fix the system,
not the template.** This is the lesson of the legacy per-body-part correctors.

## 7. Git workflow

- Branch per task: `feat/<area>-<short>`, `fix/<area>-<short>`, `port/<module>`
- Conventional commits: `feat(core): geodesic distance field over voxel grid`
- **Never commit to `main` directly. Never force-push.**
- Never commit `target/`, `node_modules/`, `dist/`, or `.app` bundles
- Commit only after §4 step 7 passes
- Push, then **observe CI green** — `gh run watch`. Never assume.

## 8. CI/CD expectations

`.github/workflows/ci.yml` on every PR and push to main:

| Job | Gate |
|---|---|
| `rust-test` | `cargo test --workspace` |
| `rust-lint` | `cargo clippy --workspace -- -D warnings` + `cargo fmt --check` |
| `arch-gate` | grep: no `tauri::` or `std::fs` inside `m2m-core` |
| `frontend` | `npm run lint` + `npm test` |
| `bench` | criterion vs `bench/baselines/`, fail on >10% regression |
| `sonar` | SonarQube quality gate |
| `build` | `cargo tauri build`, artifact size check ≤ 40 MB |

Release: tag `v*` → build signed `.app`, notarise, attach to GitHub release.

## 9. Disk discipline

`target/` grows unbounded and free disk is ~34 GB.

```bash
SZ=$(du -sm target 2>/dev/null | cut -f1); [ "${SZ:-0}" -gt 8000 ] && cargo clean
```

Run at session start and before any full rebuild. Also set in `.cargo/config.toml`:
`[profile.dev] debug = "line-tables-only"` — full debug info is the main bloat source.
