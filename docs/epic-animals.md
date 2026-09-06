# Epic: wire the animals-3d assets as app templates

Wire the 9 finished, native-rig animals from the (git-ignored) `animals-3d/output/`
into the app's template system. Standing rules: unit/integration/regression tests,
code + ponytail + sonar review, verify in the COMPILED `.app`, commit/push CI-green
with the `Co-Authored-By` trailer.

## Decisions (confirmed with the user)
- **Native-rig templates, all 9.** Keep each animal's own bone names; do NOT rename to
  the app's canonical convention. Refresh the existing 6 (buffalo/rhino/giraffe/hyena/
  elephant/crow) to the better assets + add cat/leopard/butterfly.
- **Drop `*_source_*`** raw passthrough clips.
- LFS: meshes (`assets/characters/*.glb`) are LFS; rigs + animation libraries stay
  regular git (the build/CI reads them directly and does not pull LFS).

## The four parts per animal (how the app wires a template)
- `assets/rigs/rig-<name>.glb` — skeleton-only, embedded via build.rs (globs the dir).
- `assets/animations/<name>-animations.glb` — the clips library (native bone names).
- `assets/characters/<name>.glb` — the mesh, LFS, a bundled sample.
- `crates/m2m-rig/templates/<name>.json` — the manifest (role-chains).

## Tooling
`cargo run -p m2m-pipeline --example wire_animals -- <in.glb> <rig-out.glb> <lib-out.glb>
[old_prefix new_prefix]` splits a combined `<animal>.glb` (mesh + native skeleton + all
clips) into rig + library using the app's own m2m-io — no Blender. Drops `*_source_*`,
optional clip-prefix rename. Validated on all 9 (butterfly→leopard).

Manifest role-chains come from `animals-3d/pipeline/rigmap.py` (verified native→role map)
for buffalo/rhino/giraffe/elephant/crow/cat/leopard; butterfly + hyena need bone-structure
inspection (no rigmap entry). cat == the mesh2motion `fox` skeleton, so `cat.json` = fox
chains re-pointed.

## Regression guard
`m2m-pipeline::every_manifest_matches_its_rig` validates EVERY template's chains against
its real rig (UnknownBone / DoublyClaimed / BrokenChain / RootCount). NOTE: it currently
also forbids UnclaimedBone — cat's 49 bones are all claimed, but leopard (326) and
elephant (153) carry detail bones (whiskers/toes/face) rigmap doesn't chain, so before
those, relax the guard to ALLOW UnclaimedBone (accessory-follow) and verify the fitter
tolerates unclaimed bones.

## Stages
- [x] A1. Tooling (`wire_animals` example) + proof on **cat**: rig-cat.glb (12KB) +
      cat-animations.glb (5 felid gaits `cat_*`) + cat.glb mesh (LFS) + cat.json.
      All suites green; validated in the compiled .app (fits + plays cat clips).
- [x] A2a. **leopard** wired + verified in the compiled .app (fits, plays leopard gaits,
      whiskers/coat intact). Guard relaxed to allow UnclaimedBone (326 bones, 49 chained;
      the fitter's uniform pass places the rest). Guard also caught leopard's tail Part-bones
      (fixed via traversal). Library test is now data-driven (cat + leopard).
- [ ] A2b. **butterfly** DEFERRED — structurally incompatible with the template system:
      3 root bones (all children of a non-joint armature) and no spine, so the fitter
      (needs one root + a spine) can't fit it. Options: a template-system change (allow
      multi-root/spineless), or ship it as a Proceed-only sample. Raised with the user.
- [x] A3. All 9 animals ship as ANIMATED character samples in `assets/characters/*.glb`
      (LFS, bundled): buffalo/elephant/giraffe/hyena/rhino replaced with the animals-3d
      animated versions (were 0-1 clips), crow + butterfly + cat + leopard added. Loading any
      of them → "Animate its own clips" → it animates (verified buffalo + leopard in the
      compiled .app). The `_source_` raws are hidden by the UI. butterfly is UN-deferred: it
      needs no fit/spine for own-clips. The canonical templates (rig+library+manifest) for the
      existing 6 are left as-is (they serve the fit-onto-unrigged-user-mesh path).
      Follow-up: refresh those canonical libraries to the better clips (needs a retarget pass).
- [ ] A4. Naming pass: confirm public template/clip names (cat done; check others).
- [ ] A5. (separate) Task A: improve the animal AUTOFIT + bind quality (the cat/buffalo
      fit is rough — the app re-fits + re-solves weights rather than using the asset's
      own). Task B: fine bone precision editing.

## Own-clips animation (the fix for animating rigged samples)
An already-rigged, animated model (the animal samples) now offers **"Animate its own
clips"** in the rigged-import inspector — it plays the model's OWN embedded clips directly
(`playAnimated` on the model's own bytes), no fit, no retarget. Verified on leopard:
978/978 tracks drive the mesh, it animates cleanly.

WHY this was needed: template-fit + retarget onto an ALREADY-rigged source hits a graft bug
— the export appends a second bone set with the same names, the glTF loader renames the
collision (`Root_M`→`Root_M_1`), so the animation drives one set and the skin the other →
`tracksMatchingSkin=0`, frames play but the mesh is frozen. Own-clips sidesteps it (single
skeleton). The template flow still works for an UNRIGGED user mesh (no collision).
`scene.ts playAnimated` now logs `[animate] <clip>: <driven>/<total> tracks drive the mesh`
and warns when 0 (the bug signature).

Follow-up: the template-fit-onto-already-rigged path still has the graft duplicate-bone bug
(fix graft_skin to strip the source skeleton) — lower priority now that own-clips + F1
Proceed both work.

## Clip-preview thumbnail for animals (the top moving preview)
The clip chooser's moving thumbnail (`#clip-preview`) played the animation LIBRARY on the
library character — fine for the human (its library carries a mesh) but EMPTY for animals,
whose libraries we split to skeleton-only (the 10-22 MB textured mesh lives in the LFS
character, too heavy for a hover thumbnail). `createClipPreview().load` now detects a
mesh-less library (`bounds.isEmpty()`) and draws a `SkeletonHelper`, framing from the bones'
own world positions (`boundsOfBones`, model.ts) — so the animal clip previews as a visible
moving skeleton instead of an empty box. Verified in the compiled .app (leopard template
fit → the thumbnail shows the animated leopard skeleton). own-clips mode needs no thumbnail:
its main viewport already plays the full textured model.

## Root motion (the "leopard moving up" regression)
Authored locomotion clips carry root translation; played as-is the model drifts off — worst
on the Z-up animal assets, where forward travel lands on the viewer's up axis. `scene.ts
playAnimated` now strips the root bones' `.position` tracks so the cycle plays in place.

## Leopard eyes (DONE)
Ask: "real leopard eye texture pupil sclera etc" → user chose "mostly iris big cat". Big cats
(Panthera) have ROUND pupils, not slits. Each eye is ONE sphere mesh (leopard.glb meshes
37/38, materials `leopard_iris_eye_l/_r`); was flat amber, no pupil.
Approach (worked): Blender only BAKES, never re-exports. Import leopard.glb, set the armature
`pose_position='REST'` (else the posed head rotates the world normals off the gaze), build a
geometry-driven shader on each eye material — dot(world Normal, gaze) → ColorRamp (black round
pupil ~15.6°, golden-amber iris, dark limbal edge, dark hidden back); gaze = normalize(±0.5,
-1, 0) in Blender space (glTF +Y-up→Blender +Z-up makes the leopard face -Y). Because the
shader is geometry-driven, the seamed UV doesn't matter — bake EMIT to a 1024² PNG in the
eye's UV (save Non-Color so texel bytes round-trip as sRGB baseColor). Then inject ONLY the
PNGs into leopard.glb via in-place glb edit (append to BIN, add image+bufferView+texture, set
baseColorTexture on mats 37/38, texCoord 0, baseColorFactor white) — NO re-export, so the
326-bone rig + 5 clips are byte-identical (validated: nodes/joints/anims/accessors unchanged;
own-clips still 978/978 tracks). Verified in the compiled .app: golden-amber eyes with round
pupils, coat/whiskers intact. Injection script pattern is in the session; bake settings above.

## Open questions / notes
- The app re-fits + re-binds when a user picks a template; the asset's own careful weights
  are not used (F1 "use original weights" would help). Fit quality is task A, separate from
  wiring correctness.
- `cat2` source → public name **cat** (clips `cat2_`→`cat_`). Confirm the same species
  naming for the rest as they're wired.
