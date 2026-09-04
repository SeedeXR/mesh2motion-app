# TODO: viewport regression + auto-rig UX (2026-09-04)

User report: uploaded a glb — skin colours/texture not seen in viewport; no
animation preview. Plus: add auto-fitting option, improve UX toward the Mixamo
auto-rigger flow (references/auto-rig-mixamo-reference: Orient -> Place markers
-> auto-rig; textured character throughout; moving clip-preview thumbnails).

Process per item: reproduce visually in the app FIRST, fix, then unit +
regression + integration tests, code review + ponytail review + SonarQube
review, commit/push, confirm CI green, screenshot the app to prove it.

## Diagnosis so far
- Import step DOES show model-human.glb textured (verified screenshot). load_model
  (`read_as_glb`) returns RAW glb bytes for a glTF source, so import display is
  unchanged by this session's work.
- So the regression is NOT at import for a simple textured glb. Suspects:
  animate step (preview via export_glb graft), PBR lighting (no env map), or the
  user's specific PBR/Mixamo file.

## Progress
- [x] R1 DONE (7fb80e3): texture shows at import for BOTH glb (raw bytes) and fbx
      (convert reads material -> glb::write emits it), and in the animate preview
      (graft keeps textures). Verified in the app: glb import textured, fbx import
      textured, animate preview textured.
- [x] R2 DONE (7fb80e3): animation preview no longer explodes. ROOT CAUSE was a
      frame bug in retarget_clip — target translations were raw world offsets but
      the bone nodes/source rig use the parent-local frame, so playback
      double-rotated every offset and flung the mesh into shards. Fixed to
      world_rotation[parent]^-1 * offset. ALSO: playAnimated now hides the
      octahedral fitted skeleton during playback (it hung over the moving mesh).
      Regression test asserts no bone flies >5m at frame 0. Verified in the app.
- [ ] MINOR residual: a small spike at the lead foot mid-animation (one bone/leaf).
      Investigate if quick; low priority vs the fixed explosion.
- [ ] F1 Auto-fit option + Orient step (see below).
- [ ] U1 UI/UX polish toward the reference.

## Items (original)
- [ ] R1 Texture/skin colours regression — reproduce with a PBR/Mixamo-style glb
      AND through the full flow (import->fit->bind->animate). Determine if it's
      the animate preview (export_glb output), viewport lighting for PBR (needs
      an environment map / better lights), or file-specific. Fix so BOTH glb and
      fbx show proper colours in the viewport at every step.
- [ ] R2 Animation preview regression — reproduce the animate step + clip
      thumbnails. Determine if previewAnimation (export_glb graft w/ clip) plays
      in three.js, and if the moving clip thumbnails render. Fix.
- [ ] F1 Auto-fit option — a one-click "auto-fit" after choosing a skeleton (the
      fit is already automatic; make it an explicit, smooth affordance) + an
      Orient step (rotate to face front) like the reference.
- [ ] U1 UI/UX polish toward the reference: textured character, orient controls,
      marker/guidance clarity, smooth step transitions.

## Test harness note
- Dev autoload: M2M_AUTOLOAD (import) + M2M_AUTOFIT (fit). To visually test the
  animate step, extend the harness to reach bind+animate, or drive via the app.
- Capture: swift winid + screencapture -x -o -l <id> (fresh launch = non-black).
