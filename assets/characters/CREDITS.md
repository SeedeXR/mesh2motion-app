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

## Animal samples (own-clips)

These carry the creature's **own hand-authored clips**, so loading one offers
"Animate its own clips" directly (no fit/retarget). All verified through `m2m-io`
(`rigged=true`, 0 over-influence) and in the compiled `.app` (loads, grounded on the
floor, plays its own clips).

| file | creature | bones | clips | source | licence |
|------|----------|-------|-------|--------|---------|
| `shark.glb` | shark | 33 | 7 | original mesh2motion app asset (`legacy/static/animations/shark-animations.glb`) | ships with the app |
| `bird.glb` | bird | 55 | 5 | original mesh2motion app asset | ships with the app |
| `spider.glb` | spider | 56 | 10 | original mesh2motion app asset | ships with the app |
| `horse.glb` | horse | 56 | 14 | original mesh2motion app asset | ships with the app |
| `fox.glb` | fox | 49 | 14 | original mesh2motion app asset | ships with the app |
| `dragon.glb` | dragon | 100 | 5 | original mesh2motion app asset | ships with the app |
| `snake.glb` | snake | 28 | 8 | original mesh2motion app asset | ships with the app |
| `kaiju.glb` | kaiju | 58 | 10 | original mesh2motion app asset | ships with the app |
| `fish.glb` | redfish | 19 | 1 (swim, authored) | `animals-3d/fish/redfish_text.mb` (Maya) → FBX → Blender; swim authored (carangiform) | _TODO (R-5 record)_ |
| `whaleshark.glb` | whale shark | 33 | 1 (swim, authored) | `animals-3d/whale-shark/WhaleShark.blend`; swim authored (thunniform) | _TODO (R-5 record)_ |

The 8 above from `legacy/` are the original mesh2motion app's creature assets (a
git-mv of the existing app), used here as their own-clips character samples — their
skeleton-only fit templates already ship in `assets/rigs/` + `assets/animations/`.
fish + whale shark are the two authored from scratch (aquatic undulation); fish's
`.mb` texture is not on disk, so it ships with a solid redfish material until the
source texture is recovered.

## Textures

The elephant's source rig ships untextured; the maintainer supplied a colour UV
skin (applied matte, roughness 0.95) procedural amber eyes (dark sclera, honey iris, dark pupil) baked to texture. Tusks sample the
texture's ivory hoof region; The character is
re-oriented Y-up standing on its four legs, grounded and centred (the source
asset was rotated on its side at centimetre scale). Licences ride with the R-5
asset record like the meshes.
