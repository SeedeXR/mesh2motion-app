<div align="center">

# Mesh2Motion

**Native rigging and animation for every creature.**

Rig humans, birds, fish, quadrupeds and anything else — locally, fast, no account.

</div>

---

> **Status: early port.** The shipping web app lives in [`legacy/`](./legacy) and
> still works. This root is the native Tauri + Rust rewrite, currently at
> scaffold stage. Progress is tracked in [`memory/todo.md`](./memory/todo.md).

## Why

[Mixamo](https://mixamo.com) rigs bipedal humanoids and stops there. Rigify is
powerful and unintuitive. Mesh2Motion aims at the gap: drop in *any* creature and
get a production-usable rig with clean weights in under a minute.

The legacy web app already covers 9 creature templates. Its ceiling is the
skinning solver — rigid nearest-bone assignment
([`WeightCalculator.ts:71`](legacy/src/lib/solvers/WeightCalculator.ts)), which
needs a hand-written corrector per body part per creature. The rewrite replaces it
with geodesic voxel binding, which removes that whole class of patch.

## Requirements

| | Version | Check with |
|---|---|---|
| Rust | 1.85+ | `rustc --version` |
| Node | 22+ | `node --version` |
| Xcode CLT | any recent | `xcrun --version` |
| Blender | 4.x *(optional, for the DCC bridge)* | `/Applications/Blender.app` |

macOS 12+ on Apple Silicon. Other platforms are not a current goal.

## Quick start

```bash
git clone https://github.com/SeedeXR/mesh2motion-app
cd mesh2motion-app
npm install
npm run app:dev        # launches the desktop app with hot reload
```

Build a release bundle:

```bash
npm run app:build      # → target/release/bundle/macos/Mesh2Motion.app
```

Run the checks:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npx tsc --noEmit
```

## Running the legacy web app

Kept runnable — it is the A/B correctness baseline for the new solver, not dead code.

```bash
cd legacy
npm install
npm run dev
```

## Layout

```
crates/
  m2m-core      pure geometry + skinning solver — no I/O, no Tauri
  m2m-io        FBX, glTF, GLB read/write
  m2m-rig       creature templates, fitting, retargeting, bone auto-mapping
  m2m-bridge    Blender DCC bridge
src-tauri/      thin Tauri command layer — no algorithms
app/            frontend: TypeScript + Three.js viewport
legacy/         original web app (reference implementation + test baseline)
memory/         project documentation and agent working memory
references/     Mixamo FBX corpus used for round-trip testing
```

## Where to start reading

1. [`memory/project_context.md`](memory/project_context.md) — what this is and why
2. [`memory/architecture.md`](memory/architecture.md) — system design + decision record
3. [`memory/porting.md`](memory/porting.md) — how the legacy app works
4. [`memory/todo.md`](memory/todo.md) — the roadmap

## Licences

Code under [MIT](LICENSE-MIT.MD). Art assets (models, rigs, animations) under
[CC0](LICENSE-CC0.MD). Any newly added asset must record its provenance and
licence.
