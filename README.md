<div align="center">

# Mesh2Motion

**Native rigging and animation for every creature.**

Rig humans, birds, fish, quadrupeds and anything else — locally, fast, no account.

</div>

---

> **Status: the core rigging flow is complete.** All six steps — import, skeleton,
> fit, bind, animate, export — work end-to-end for every one of the 9 creature
> templates, verified against Blender ([`src-tauri/tests/visual_regression.rs`](src-tauri/tests/visual_regression.rs)).
> Remaining work is polish and release (signing, docs, a perf pass); see
> [`memory/todo.md`](./memory/todo.md). The original web app lives in
> [`legacy/`](./legacy) and still runs — it is the A/B correctness baseline.

## Why

[Mixamo](https://mixamo.com) rigs bipedal humanoids and stops there. Rigify is
powerful and unintuitive. Mesh2Motion aims at the gap: drop in *any* creature and
get a production-usable rig with clean weights in under a minute.

The legacy web app's ceiling was its skinning solver — rigid nearest-bone
assignment ([`WeightCalculator.ts:71`](legacy/src/lib/solvers/WeightCalculator.ts)),
which needed a hand-written corrector per body part per creature. The native
rewrite replaces it with geodesic voxel binding ([`crates/m2m-rig`](crates/m2m-rig)),
which removes that whole class of patch: across all 9 templates every vertex binds
with a full unit of weight and no unreachable islands.

## How it works — the six steps

The app is a straight-line pipeline; each step is one Tauri command over the Rust
core ([`src-tauri/src/lib.rs`](src-tauri/src/lib.rs)):

| Step | What you do | Behind it |
|---|---|---|
| **1. Import** | Drop in a mesh (`.glb`, `.fbx`) — existing bones are kept, not stripped | `import_model` |
| **2. Skeleton** | Pick one of 9 creature templates (human, bird, fish, quadruped, …) | `skeleton_templates` |
| **3. Fit** | The template snaps to your mesh's proportions | `fit_skeleton` |
| **4. Bind** | Geodesic voxel binding computes skin weights; a weight-paint overlay shows them | `bind_weights`, `weight_overlay` |
| **5. Animate** | Pick a clip, preview it live, retargeted onto your rig | `animation_clips`, `preview_animation` |
| **6. Export** | Write `.glb` (glTF binary) or `.fbx` — mesh + skeleton + weights + clip | `export_model` |

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
cargo test --workspace                                  # Rust unit + integration
cargo clippy --workspace --all-targets -- -D warnings   # lint, warnings = errors
npx tsc --noEmit && npx vitest run                       # frontend typecheck + tests
```

`#[ignore]`d tests need a local Blender (they read exports back through it) and are
skipped by default. Run the full visual-regression sweep with:

```bash
cargo test -p mesh2motion --release -- --ignored
```

## The Blender bridge

[`crates/m2m-bridge`](crates/m2m-bridge) inspects a model by importing it into a
headless Blender — the one reader in the project that shares none of our design, so
it is the independent check that an export is correct. It resolves Blender from
`$M2M_BLENDER` or the macOS default, and is optional: nothing in the core flow
depends on it.

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
