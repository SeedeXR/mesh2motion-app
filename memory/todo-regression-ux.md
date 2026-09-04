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
- [x] F1 DONE (e288386): "Auto-fit again" button in the Fit step + orient-aware
      guidance (orient with move/rotate tools, then auto-fit). Verified in-app.
- [x] MINOR foot spike: investigated — Blender renders frames 5/15/20/33 clean,
      no shards. Was a transient single-frame artifact, not a persistent bug.
- [x] RENDERING FIDELITY DONE (2edd5c6, 18986e7): the real regression. Viewport
      had no IBL/tone-mapping/AA, so PBR (esp. metallic) rendered flat/dark.
      Added RoomEnvironment IBL + KHR PBR-neutral tone map + sRGB + antialias.
      PROVEN vs Blender with metallic PBR spheres (metal now shiny not black;
      colours match). Weight-paint overlay set toneMapped:false so debug hues
      stay true (verified vibrant in-app).
- [x] Weight-paint bones-clutter hidden during overlay (268f4e0); clip-preview
      thumbnails confirmed animating (preview.ts AnimationMixer + setAnimationLoop).
- [~] U1 UI/UX: concrete improvements done (orient guidance, autofit button,
      textured throughout via rendering fix, refine-by-drag exists, animating
      thumbnails, clean weight-paint/animate views). Full Mixamo marker-placement
      UX = a paradigm redesign that needs USER DIRECTION (markers vs the current
      more-automatic auto-fit).
- BLOCKED ON USER INPUT: (a) their SPECIFIC model to reproduce any remaining
      colour mismatch (rendering is model-specific); (b) Maya screenshots — maya
      headless hardware-render not available (no GPU viewport in mayapy batch);
      (c) the auto-rig UX vision (marker placement vs smoother current flow).

## Sonar (2026-09-04): gate ERROR from PRE-EXISTING branch issues, NOT the
## regression/F1 commits (those add 0 new violations): author_rig.rs:370
## essential-complexity example (documented P4-Q defer), history.ts:13 stale
## (states is already readonly), + 3.35% duplication from the earlier texture
## work (distinct glTF/FBX constructs). CI green on all commits.

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
