# Plan: keep UVs / normals / materials / textures on export (P?-new)

**Problem (verified):** export strips NORMAL, TEXCOORD_0, materials, textures.
The pipeline reduces every model to positions+indices in `mesh_of`
(`crates/m2m-pipeline/src/lib.rs`), and the custom glb writer explicitly drops
shading (`glb/write.rs:12`). Round-trip proof: source `model-human.glb` has
`[NORMAL, POSITION, TEXCOORD_0]` + 1 material + 1 image; exported rig has
`[JOINTS_0, POSITION, WEIGHTS_0]` + 0 materials + 0 images.

**User decision:** fix BOTH glb and fbx now.

## Phase 1 — GLB via GRAFT (augment the original, custom writer untouched)
`model-human.glb` is clean: 1 node (identity), 1 primitive
POSITION/NORMAL/TEXCOORD_0, 1 material→1 texture→1 image.
- New `crates/m2m-io/src/glb/graft.rs`: `graft_skin(model, bones, per_primitive_weights, animation)` — load original via `gltf::binary::Glb::from_slice` → `json::Root` + bin, APPEND JOINTS_0/WEIGHTS_0 per triangle primitive (read order), IBM accessor, skeleton nodes, skin; set mesh-owning node.skin; add skeleton root to scene; optional animation (samplers+channels). Repack glb. Original materials/textures/images/UV/normals never touched → preserved by construction.
- Bind math: mesh node is identity for the target, so IBM = inverse(jointWorld) exactly as `rigged_document`. Fold meshNodeWorld into IBM for the general (non-identity) case.
- Alignment: weights from `solve()` are over the merged mesh in `document.primitives` order (gltf.meshes()→primitives(), triangles). Graft iterates the SAME order; assert Σ vertex counts == solve mesh vtx count, else error (no silent misalignment).
- Wire `export_glb` to call graft. `overlay_glb` KEEPS the custom writer (synthetic colors, no texture needed).
- Verify: MCP export human → attrs include TEXCOORD_0 + image; open in Blender headless, confirm texture present. Then animated export keeps the clip.

## Phase 2 — FBX (thread through the custom builder)
`crates/m2m-io/src/fbx/*` (build.rs, geometry.rs, encode.rs, binary.rs). Add:
LayerElementNormal, LayerElementUV (+ LayerElementMaterial), Materials, and an
embedded baseColor texture (Video node w/ Content = PNG bytes, Texture node,
Connections). `build::Mesh` gains normals+uv; `build::Scene` gains materials +
textures. Map per-vertex (mesh_of concat order). Verify in Maya (strict) + Blender.

## Status
- [x] Phase 1 DONE: `glb/graft.rs` grafts skin+skeleton+animation onto the source glTF (serde_json Value + BIN append, custom writer untouched). `export_glb` grafts for a glTF source, rebuilds for FBX source. VERIFIED: exported human keeps NORMAL/TEXCOORD_0 + material 'Main' + image 'color-palette' 499x500 (Blender confirms UV+material+texture connected); rig valid (66 bones/1 armature/7399 weighted); animated export Blender-reads action Chest_Open range 0-33. 4 export tests updated (bone→node via skin.joints) + 1 new shading test. All pipeline/io/mcp tests + clippy green.
- [x] Phase 2 DONE: glb READER now carries NORMAL/TEXCOORD_0/material+embedded image (Primitive.normals/uvs/material, Document.materials, Material{base_color_factor, base_color_image:Image{data,mime}}). FBX BUILDER emits LayerElementNormal + LayerElementUV (ByVertice/Direct, name "map1", V flipped 1-v) + LayerElementMaterial (AllSame) + Layer; Material(phong) + Texture(TextureVideoClip) + Video(Content=embedded PNG) + connections (material→model OO, texture→material OP DiffuseColor, video→texture OO); definitions counts. export_fbx threads via fbx_source_shading (merged vertex order, normals by node normal-matrix; first material for multi-material). VERIFIED both engines: BLENDER uv=map1, material Main, packed image 499x500; MAYA (strict) uvsets=[map1] uvcoords=7399, material Main, embedded texture extracted to .fbm file-node; texture renders correctly on body (V-flip right). FBX imports lying-down in Blender = pre-existing no-UpAxis GlobalSettings quirk (Maya upright, unrelated to shading). +1 fbx shading test. clippy + all tests green.
      KNOWN LIMITATIONS (follow-ups): see the stepwise loose-ends below.

## Loose-ends loop (2026-09-04)
- [x] ITEM 2a DONE (a1beb75): FBX-source exports keep UVs + normals (both fbx and glb). convert.rs weld carries first-corner normal+UV (V-flipped), glb writer emits NORMAL/TEXCOORD_0 (additive), fbx_source_shading reads via import::load, rigged_document takes shading. Verified Blender (map1 UVs+normals for fbx, NORMAL/TEXCOORD_0 for glb); convert test asserts a normal+UV per welded vertex; glb+convert fuzzed 60s each, no panics.
- [~] ITEM 4 MOOT (verified): `.gltf` (JSON, starts `{`) is NOT an import format — import::read_any accepts only glTF-magic (glb) or FBX-binary; a `.gltf` goes to the fbx parser and fails. So there is no `.gltf` source for the graft; the glb-magic gate exactly matches the import surface. Nothing to do.
- [~] ITEM 1 (FBX Blender orientation) — MISCHARACTERIZED + DEFERRED. GlobalSettings ALREADY declares UpAxis=Y (build.rs global_settings_node). The real issue: Blender's FBX importer rotates the ARMATURE object +90°X (Y-up→Z-up) but leaves the skinned MESH object at identity, so the mesh lies down while the armature stands. Pre-existing (before this session), Maya-unaffected (imports upright). A safe fix reworks the mesh/armature hierarchy in the Maya-verified fbx builder (e.g. parent the mesh under the armature root with a compensating local transform so Blender rotates them together) and MUST re-verify the bind in Maya — not a one-line/loop-safe change. Deferred with this reason rather than risk the verified bind for a cosmetic Blender-only rest-pose display issue.
- [x] ITEM 2b DONE (040a52f): FBX material/texture reader. convert.rs read_material follows Model<-Material<-Texture<-Video, reads embedded Content bytes (via Object.node raw tree), PNG/JPEG by signature, dedup by id; primitive.material set; Document.materials populated. fbx_source_shading already reads Document.materials so export_fbx re-emits with NO further change. VERIFIED: our textured FBX -> import -> rig -> export FBX keeps material Main + packed 499x500 texture + map1 UVs (Blender). Self-contained round-trip test asserts exact embedded PNG bytes survive build->encode->parse->convert. fbx_pipeline fuzzed 75s/480k, no panic.
- [~] ITEM 3 (multi-material FBX) — NON-IMPACTING, deferred. Checked every shipped asset: all are 1 (or 0) materials — NONE is multi-material. glTF->glb already keeps all materials via grafting; glTF/FBX->fbx merges to one mesh w/ the first material, which is exactly correct for every real asset. A fix (per-polygon LayerElementMaterial + multiple Material/Texture/Video) has no fixture and no user-facing impact today; deferred until a multi-material asset exists.
- BOUNDARY (not a listed item): FBX->glb drops the material (the glb REBUILD writer emits no materials; glTF sources keep them by grafting, and FBX->fbx keeps them by re-emit). Fixing needs the custom glb writer to emit materials+embedded images — the writer extension the graft was built to avoid. Low priority: FBX->fbx is the natural round-trip and is full.

## FINAL STATE (texture passthrough)
Export shading matrix — what each path keeps:
- glTF(.glb) -> glb: FULL (materials+textures+UVs+normals, via graft)
- glTF(.glb) -> fbx: FULL (via fbx_source_shading re-emit, Maya+Blender verified)
- FBX -> fbx:        FULL (materials+textures+UVs+normals, this loop)
- FBX -> glb:        UVs+normals (material dropped — boundary above)
Deferred: Item 1 (Blender rest-pose orientation, risky bind rework, Maya-fine), Item 3 (multi-material, no asset uses it), FBX->glb material (glb writer extension).

LESSON: mesh node of model-human is identity; POSITION/NORMAL/TEXCOORD_0 accessors 0/1/2. import is glb+fbx only (no .gltf).
