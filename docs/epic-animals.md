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
- [ ] A2. Add **leopard**, **butterfly** (new animals). Relax the manifest guard for
      unclaimed detail bones; author butterfly chains from its 3-bone structure.
- [ ] A3. Refresh existing 6 (**buffalo, rhino, giraffe, hyena, elephant, crow**) to the
      native rigs+libraries. Rewrite their manifests to native names; update the fit tests
      that assert the old canonical bone names/counts.
- [ ] A4. Naming pass: confirm public template/clip names (cat done; check others).
- [ ] A5. (separate) Task A: improve the animal AUTOFIT + bind quality (the cat/buffalo
      fit is rough — the app re-fits + re-solves weights rather than using the asset's
      own). Task B: fine bone precision editing.

## Open questions / notes
- The app re-fits + re-binds when a user picks a template; the asset's own careful weights
  are not used (F1 "use original weights" would help). Fit quality is task A, separate from
  wiring correctness.
- `cat2` source → public name **cat** (clips `cat2_`→`cat_`). Confirm the same species
  naming for the rest as they're wired.
