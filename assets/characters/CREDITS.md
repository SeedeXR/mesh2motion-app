# Shipped character rigs — provenance (R-5)

Each `.glb` here is a **complete rigged character** — original mesh + skeleton +
skin weights — bundled into the app (`characters/` in the .app, via
`tauri.conf.json` → `bundle.resources`). Unlike the skeleton-only fit templates
in `assets/rigs/`, these carry the creature's **original mesh**, so a user can
take the whole character (rig + skin) or, via the app's export, the skeleton
alone (rig only) — the Mixamo-style choice.

## Licence basis

Sourced and licence-checked by the maintainer under R-5 (`docs/research/asset-sourcing.md`):
all assets are asserted **free for commercial use / royalty-free**, redistributable
in a shipped product. The source assets carry no embedded licence file, so the
per-asset `source_url` / `author` / exact licence below is **to be completed by
the maintainer from the R-5 sourcing record** before public release — the fields
are recorded here so provenance is traceable, not lost.

## Pipeline (reproducible)

Maya/`.ma`·`.mb` → `mayapy` `FBXExport` (lights/cameras stripped) → Blender
headless FBX→glb (`export_scene.gltf`, caps skin to 4 influences) → verified with
`examples/read_glb` + a Blender render. FBX sources skip the first step; `.blend`
sources import straight into Blender. Scripts in the session scratchpad.

## Characters

| file | creature | body plan | verts | bones | source format | source_url | author | licence | retrieved |
|------|----------|-----------|-------|-------|---------------|-----------|--------|---------|-----------|
| `elephant.glb` | Asian elephant | large quadruped (trunk, tusks, ears, tail) | 33 746 | 153 | `ElephantRig_v2.ma` (Maya 2014) | _TODO (R-5 record)_ | _TODO_ | free/commercial (asserted) | 2026-09 |
| `rhino.glb` | Southern white rhino | large quadruped (horn) | 33 743 | 35 | `model-56a-southern-white-rhino` (glb, textured) | _TODO (R-5 record)_ | _TODO_ | free/commercial (asserted) | 2026-09 |
| `giraffe.glb` | Giraffe | long-neck quadruped | 2 322 | 48 | `Giraffe_.blend` | _TODO (R-5 record)_ | _TODO_ | free/commercial (asserted) | 2026-09 |
| `buffalo.glb` | African buffalo | large bovine quadruped (horns) | 28 637 | 42 | `african buffalo.glb` (textured) | _TODO (R-5 record)_ | _TODO_ | free/commercial (asserted) | 2026-09 |
| `hyena.glb` | Spotted hyena | canine/quadruped (sloped back) | 9 276 | 122 | `HYENA_DEMO.fbx` | _TODO (R-5 record)_ | _TODO_ | free/commercial (asserted) | 2026-09 |

Verified: imports through `m2m-io` (glb reader, clean report — 0 over-influence,
0 non-finite), renders as an intact deforming character in Blender 5.2.

_New rows are added as each creature is prepared and verified._

## Textures

The elephant's source rig ships untextured; two UV skins for it were supplied by
the maintainer and applied as two matte variants: `elephant.glb` (natural
colour, default) and `elephant-dark.glb` (dark hide). The tusks sample the
texture's ivory hoof region. Licences ride with the R-5 asset record like the
meshes.
