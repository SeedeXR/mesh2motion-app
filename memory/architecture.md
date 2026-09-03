# Architecture

**Decision (2026-08-29, user-confirmed): Rust compute core + Three.js viewport in
the Tauri webview.** Verified on the reference machine: the webview resolves to
**WebGPU**, so the viewport is Metal-backed without a native renderer (ADR A1a). All heavy math is native; the viewport keeps Three.js so the
~3,000 LOC of working gizmo/picking/skeleton-helper code survives the port. The
renderer stays swappable — nothing in `m2m-core` knows what draws it.

## 1. System overview

```mermaid
flowchart TB
    subgraph WV["WKWebView — app/ (TypeScript)"]
        UI["UI shell<br/>dark theme · Lucide · Asta Sans"]
        VP["Viewport<br/>Three.js · WebGL2/WebGPU"]
        ST["State store<br/>step machine"]
        UI <--> ST
        VP <--> ST
    end

    subgraph TS["src-tauri/ — Tauri shell"]
        CMD["command handlers"]
        EV["event bus"]
    end

    subgraph RS["crates/ — Rust workspace"]
        CORE["m2m-core<br/>skinning · voxel · geodesic · mesh"]
        IO["m2m-io<br/>FBX · GLTF · GLB"]
        RIG["m2m-rig<br/>templates · fitting · retarget · automap"]
        BR["m2m-bridge<br/>Blender DCC"]
    end

    BL[("Blender<br/>headless / live")]

    ST <-->|"IPC: JSON control<br/>+ ArrayBuffer payloads"| CMD
    CMD --> CORE
    CMD --> IO
    CMD --> RIG
    CMD --> BR
    EV -->|progress · logs| ST
    BR <-->|JSON-RPC over stdio| BL
    RIG --> CORE
    IO --> CORE
```

## 2. Crate boundaries

| Crate | Depends on | Must NOT depend on | Responsibility |
|---|---|---|---|
| `m2m-core` | `glam`, `rayon` | tauri, serde_json, any I/O | pure geometry + solver. Plain data in, plain data out. |
| `m2m-io` | `m2m-core` | tauri | FBX read/write, GLTF/GLB read/write |
| `m2m-rig` | `m2m-core` | tauri | template defs, skeleton fitting, retargeting, bone auto-mapping |
| `m2m-bridge` | `m2m-io` | tauri | Blender process control + RPC |
| `src-tauri` | all | — | thin command layer. **No algorithms here.** |

**The rule that keeps this testable:** `m2m-core` is a library you can benchmark
from a `criterion` harness with no window, no webview, and no Tauri runtime. If a
change to `m2m-core` needs a running app to test, the change is in the wrong crate.

## 3. The skinning pipeline (new)

Replaces the legacy nearest-bone chain documented in `porting.md` §3.

```mermaid
flowchart TD
    M[mesh + fitted skeleton] --> V["1· sparse voxelisation<br/>interior/exterior classification"]
    V --> G["2· geodesic distance field<br/>per bone, through voxel interior<br/>(rayon, per-bone parallel)"]
    G --> W["3· weights from geodesic falloff<br/>k-nearest bones, k≤4"]
    W --> B{"quality mode"}
    B -->|fast| N
    B -->|high| BH["3b· robust biharmonic refinement<br/>mesh-free geometric fields"]
    BH --> N["4· normalise, sum=1.0"]
    N --> P["5· prune: root + leaf bones → 0"]
    P --> OUT([skin_indices u16×4, skin_weights f32×4])
```

**Why geodesic, not Euclidean.** Distance measured *through the mesh interior*
cannot jump across empty space. A hand resting near a hip is far away geodesically
even though it is close Euclidean. This single change removes the need for
`ExtremityWeightCorrector` and `ArmWeightCorrector` — they were patches for the
Euclidean failure mode. **Deleting them is a deliverable, not a side effect.**

**Why voxel, not tetrahedral.** Bounded Biharmonic Weights needs a volumetric tet
mesh, which fails on the non-watertight, self-intersecting meshes artists actually
have. Sparse voxelisation degrades gracefully. This is the documented reason Maya
ships Geodesic Voxel Binding.

References (verified 2026-08-29):
- Dionne & de Lasa, *Geodesic Voxel Binding for Production Character Meshes*, SCA 2013
- Dodik, Sitzmann, Solomon, Stein, *Robust Biharmonic Skinning Using Geometric Fields*, TOG 2025 — arXiv:2406.00238
- Jacobson et al., *Bounded Biharmonic Weights for Real-Time Deformation*, SIGGRAPH 2011

## 4. IPC contract

Two channels, deliberately separate:

| Channel | Carries | Mechanism |
|---|---|---|
| **Control** | commands, params, results, errors | Tauri `invoke`, serde JSON |
| **Bulk** | vertex/index/weight buffers | raw `ArrayBuffer` — never JSON |

**Never serialise a vertex buffer as JSON.** A 50k-vertex mesh is ~1.2 MB binary and
~9 MB as a JSON number array, and the parse cost dominates the solve. Buffers cross
as bytes with a small JSON header describing layout.

Long operations emit progress events rather than blocking:

```
invoke("bind_skin", {mesh_id, skeleton_id, quality})  →  job_id
event "m2m://progress"  {job_id, stage, pct}
event "m2m://done"      {job_id, buffer_handle}
```

## 5. Frontend structure (`app/`)

```
app/src/
  main.ts              entry, step machine (ports Mesh2MotionEngine.ts)
  ipc/                 typed wrappers over invoke — the ONLY place invoke is called
  viewport/            Three.js scene, camera, gizmos (ported from legacy/src/lib)
  steps/               one module per ProcessStep
  ui/                  components, Lucide icons, dialogs
  state/               single store; UI is a view over it
```

`ipc/` being the sole `invoke` caller is what makes the Rust boundary mockable in
frontend tests.

## 6. Blender bridge

```mermaid
sequenceDiagram
    participant A as m2m app
    participant B as m2m-bridge
    participant BL as Blender
    A->>B: send_to_blender(rig, anims)
    B->>B: write .glb to temp
    B->>BL: spawn -b --python bridge.py<br/>(or connect to live session)
    BL->>BL: import, apply, report
    BL-->>B: JSON-RPC result over stdio
    B-->>A: event m2m://bridge-result
```

Two modes: **headless** (spawn `/Applications/Blender.app/Contents/MacOS/Blender -b`,
used for automated visual regression in CI) and **live** (attach to a running
Blender with the companion add-on, for artist round-tripping).

## 7. Threading and memory

- Solver work runs on a `rayon` pool sized to `num_cpus - 1`, leaving a core for UI.
- The webview thread never blocks; every command over ~16 ms returns a `job_id`.
- Large buffers are owned by Rust and handed to JS as views. One owner, no copies.
- Voxel grids are the peak allocation; grid resolution is adaptive to the vertex
  budget so peak RSS stays inside the 1.5 GB budget in `philosophy.md`.

## 8. Security

- Tauri CSP locked down; no remote content loaded into the webview.
- Filesystem scope limited to user-selected paths via the dialog plugin.
- FBX/GLTF parsers are **hostile-input boundaries** — a malformed file must error,
  never panic, never OOM. Fuzz targets required (`test.md` §5).
- No telemetry, no network calls at runtime. Fonts and assets are vendored.

## 8a. Prior art — Blender's Rigify

*Researched session 026, at the user's request. Read from the addon shipped with
the installed Blender (`/Applications/Blender.app/Contents/Resources/5.2/scripts/addons_core/rigify`);
the 2.81 manual URL returns 403, and the source is the better reference anyway.*

### Licence — read this first

**Rigify is `GPL-2.0-or-later`. mesh2motion is `MIT` (`Cargo.toml:15`).**

- **Do not** copy Rigify code into this repo.
- **Do not** copy or redistribute its metarig data. The bone coordinates in
  `metarigs/Animals/*.py` are the creative content of a GPL file, so a template
  derived from them cannot ship under MIT.
- **Do** reimplement architecture and taxonomy. Ideas and structure are fair to
  learn from; that is not the same as copying expression.
- **Do** use Blender+Rigify locally as a dev-time comparison tool, provided
  nothing generated from GPL metarig data is committed.

### What it is

Two tiers, which is the insight worth taking:

1. **Metarig** — a small, plain, editable armature the user fits to the mesh.
   Every bone carries a `rigify_type` (`limbs.paw`, `spines.basic_spine`, ...)
   plus per-type `rigify_parameters`. The template is *data*: "this chain is a
   paw, that chain is a tentacle."
2. **Generated rig** — a 10-stage pipeline (`initialize`, `prepare_bones`,
   `generate_bones`, `parent_bones`, `configure_bones`, `preapply_bones`,
   `apply_bones`, `rig_bones`, `generate_widgets`, `finalize`) expands each
   tagged chain into deform (`DEF-`), organizational (`ORG-`), mechanism
   (`MCH-`) and control bones, with constraints, drivers and widgets. The
   phases are strict because chains cross-reference each other.

43 rig-type modules. Metarig sizes: human 159 bones, wolf 190, cat 174, bird 75,
horse 70, shark 35, basic_human 29, basic_quadruped 34.

**A wing is not a wing solver.** The bird metarig's 75 bones are tagged
13x `limbs.simple_tentacle`, 2x `limbs.paw`, and one each of
`spines.super_head` / `basic_tail` / `basic_spine`, plus 20x `basic.super_copy`.
Composition of generic chains, not a bespoke type per species.

### Why it matters here

The species overlap with our own templates is near total:

| Rigify | ours (bone counts measured session 025) |
|---|---|
| `human`, `basic_human` | `rig-human` (66) |
| `bird` | `rig-bird` (55) |
| `cat`, `wolf` | `rig-fox` (49) |
| `horse` | `rig-horse` (56) |
| `shark` | `rig-shark` (33) |
| — | `rig-snake` (28), `rig-spider` (56), `rig-dragon` (99), `rig-kaiju` (58) |

We already have the species. What we lack is the **typed chain**: our templates
are flat bone lists in a `.glb`, with nothing saying "these five bones are a
digitigrade leg."

### What to take, ranked

1. **A typed-chain template format** (see A7 below). Highest value, and the
   timing is right: `crates/m2m-rig/src/lib.rs` is a 15-line stub, so there is
   nothing to rewrite.
2. **The rig-type taxonomy as a topology specification** — not the code, the
   list: `limbs/{arm,leg,paw,front_paw,rear_paw,simple_tentacle,spline_tentacle,super_finger,super_palm}`
   and `spines/{basic_spine,basic_tail,super_head}`. It is a tested answer to
   "what kinds of limb actually exist across vertebrates". `paw.py` carrying an
   optional **second heel control** is the digitigrade detail that separates a
   fox rig that works from one that does not.
3. **The naming discipline** (`utils/naming.py`: `org()`, `mch()`, `deformer()`,
   `strip_org()`, `make_derived_name()`). A systematic prefix scheme separating
   what deforms the mesh from what is only mechanism. Needed the moment we add
   IK for viewport posing, because **only deform bones may be skinned and
   exported** — getting that boundary wrong ships files with several times the
   bones a DCC expects.

### What to skip

Widgets and the generated Python IK/FK UI. They are Blender-only and
unexportable: neither FBX nor glTF carries constraints or drivers, so a control
rig cannot survive export at all. We export a deform skeleton plus baked
animation, which is the correct Mixamo-like model. **Rigify's value to us is
template structure and limb taxonomy, not rig generation.**

## 9. Architecture decision record

| # | Date | Decision | Rationale |
|---|---|---|---|
| A1 | 2026-08-29 | Rust core + Three.js viewport (not full wgpu) | preserves ~3k LOC of working interaction code; renderer stays swappable; fastest path to usable |
| A1a | 2026-08-29 | **Verified: WKWebView on macOS 26.6.2 resolves to WebGPU**, not the assumed WebGL2 fallback | measured by requesting a real adapter inside the shipped app (`[m2m] render backend: webgpu`). The viewport therefore reaches Metal through WebGPU with no native renderer, which strengthens A1 rather than weakening it. The backend is detected at runtime and shown in the status bar — never assumed per-machine. |
| A2 | 2026-08-29 | Geodesic voxel binding as default solver | removes the Euclidean failure mode that forced 3 per-body-part correctors; robust on non-watertight meshes |
| A3 | 2026-08-29 | Port the legacy FBX parser to Rust rather than use `fbxcel` | verified: `fbxcel` is binary-only, read-only, no ASCII, no export; `fbxcel-dom` is v0.0.6 |
| A4 | 2026-08-29 | Drop hand-rolled `Quat`/`Vec3`/`Transform`, use `glam` | 1,300 LOC deleted; `glam` is SIMD-optimised and battle-tested |
| A5 | 2026-08-29 | Neural rigging (UniRig) deferred to opt-in phase | ~1.5 GB RAM + ORT-from-source build conflicts with the resource budget; templates + geodesic solve first |
| A6 | 2026-08-29 | Binary IPC for buffers, JSON only for control | JSON vertex arrays are ~7× larger and parse-dominated |
| A7 | 2026-08-31 | **Templates become typed chains, not flat bone lists** — a template declares chains with a kind (`spine`, `tail`, `neck`, `digitigrade_leg`, `plantigrade_leg`, `wing`, `finger`, `tentacle`) and per-kind parameters | Structure learned from Rigify (§8a), reimplemented, never copied — it is GPL and we are MIT. Pays off three ways: fitting gets per-kind rules (a paw needs a ground plane and a heel pivot, a spine needs a centreline); retarget becomes chain-to-chain rather than name-to-name, superseding most of the legacy's 32-category `bone-automap` name guessing; and adding a species becomes data rather than code, which is what "intuitive to rig different kinds of animals" (O2) actually requires. `m2m-rig` is still a stub, so this costs no rewrite. |
