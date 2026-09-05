# Epic: rig follow-ups (F1 skip-rigging, F2 animal precision)

Follow-ups after the mixamo-UX epic. Same standing rules: unit/integration/regression
tests, code + ponytail + sonar review, verify visually in the COMPILED `.app`,
commit/push CI-green with the `Co-Authored-By: Claude Opus 4.8` trailer.

User decisions (confirmed):
- **F1** "Proceed" = **animate the imported rig with OUR clips** (auto bone-mapping),
  jump to Animate. "Re-rig" = the normal template flow. Humanoid imports retarget from
  the human library; non-humanoid may have no matching motion (content limit).
- **F2** animals: **no markers** (autofit only, animals are complex). Surface the
  autofit → edit-bones → resolve flow, and use **renders of our own rigged animal
  sample models** as the reference image (buffalo/giraffe/… are rigged, 42–48 joints).

Feasibility (verified, see session notes):
- `preview_animation`/`export_model` accept ANY `FittedSkeleton`; they only use the
  `template` string to pick a clip library, never re-fit.
- No path builds a `FittedSkeleton` from an imported rig yet — but `retarget_source`
  already extracts names/parents/world-positions/local-rest-rotations from a glb skin.
- `retarget_clip` maps library→target by EXACT name today (empty map → no motion for a
  non-template rig). `automap::map_bones_best` (Mixamo/Rigify name tables in
  `crates/m2m-rig/known-rigs/*.json` + structural fallback) exists but is unused in the
  app. `retarget_clip` already builds both source & target as `automap::Skeleton`.

## Stages

- [x] S1 (F2 nav). `runFit(name, advance)` — `chooseTemplate` passes `advance` for the
      no-marker (animal) path so it lands on the Fit step. Verified in compiled .app: buffalo
      shows the fitted skeleton + draggable joint handles + "Auto-fit again". (Note: the animal
      autofit placement is rough — a reason S5's reference should be the OWN rig, not autofit.)
- [x] S2 (F1 backend a). `skeleton_from_import(model) -> FittedSkeleton` + Tauri command
      `skeleton_from_import` + IPC `skeletonFromImport`. Reads the model's own skin (names,
      parents, world positions, local rest rotations). Tested on buffalo (rigged, forest of
      IK+deform roots) and rejects the unrigged human mesh.
- [x] S3 (F1 backend b). `retarget_clip`: exact-name mapping primary; when it covers < half
      the source bones, fall back to `map_bones_best(source, target, known_rigs(), 0.5)` (take
      it only if it maps more). `known_rigs()` embeds mixamo/rigify JSON via include_str!.
      Tested: 47 pipeline tests still pass (templates unregressed) + a new end-to-end test
      proving a `mixamorig:`-named import retargets a human clip (motion actually transfers).
- [x] S4 (F1 frontend). Rigged-import inspector: "Proceed to Animate" / "Re-rig from template".
      Proceed → `skeletonFromImport` → bind → chosen='human' → jump to Animate. Verified in the
      compiled .app: the Mixamo fixture imported, skipped fitting, and PLAYS Chest_Open
      retargeted onto its own mixamorig skeleton (clean deformation). (M2M_AUTOPROCEED dev hook.)
- [ ] S5 (F2 renders). Render each rigged animal sample (app capture: load → autofit →
      mesh+skeleton) to a reference image; show it in the animal Fit step (a
      MARKER_GUIDES-equivalent for animals). Verify in compiled .app.

Known simplifications / follow-ups:
- Proceed recomputes weights with our solver (not the import's original weights). Fine for
  v1; using original weights is a later quality pass.
- Non-humanoid imports: Proceed-to-Animate only offered when a library maps; else keep
  Re-rig (or Export-as-is) — decide in S4.
