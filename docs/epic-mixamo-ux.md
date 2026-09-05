# Epic: Mixamo-style UX (items 1–3)

Driven by a 60s-paced `/loop`. Each iteration: do the next unchecked step, verify
in the COMPILED build (not just dev) + tests, commit with CI green, update this
file. Stop when items 1–3 are all done. **Item 4 (solver-vs-Mixamo) is NOT built**
— reported as "defer, solver is good".

References: `references/auto-rig-mixamo-reference/` (mixamo-skeleton.png,
mixamo-animation-extra-features.png, mixamo-export-modal-options.png,
mixamo-autorig-research.md). Golden test case: `e2e/mannequin-unrigged-markers.json`
(marker solve validated: symmetric, grounded, spine-ordered).

Standing rules: unit/integration/regression tests; code + ponytail + sonar review;
verify visually in the compiled `.app`; commit/push CI-green; Co-Authored-By trailer.

---

## Item 1 — Mixamo-style skeleton visualization

Replace the subtle grey octahedra with the Mixamo look (mixamo-skeleton.png): a
prominent, colored (blue→purple gradient down the hierarchy) tapered bone per
joint→child, for humans AND animals. Keep the sphere joint handles for precision
dragging, and add the placement loupe (zoom preview) when adjusting a handle.
Learn from the legacy `SkeletonHelper` lines (thin) — go beyond them.

- [ ] 1a. Survey current rendering: `skeletonOctahedra`, `fittedSkeleton` material/color, `jointHandles` (scene.ts). Decide shape (elongated pyramid) + gradient coloring by hierarchy depth.
- [ ] 1b. Color the bones with a hierarchy-depth gradient (Mixamo blue→purple); make them read as the primary skeleton. Applies to fitted + imported skeletons, human + animal templates.
- [ ] 1c. Add the zoom loupe to joint-handle drag (reuse the marker loupe: setLoupeTarget on handle grab/drag).
- [ ] 1d. Verify compiled build (human + one animal template) + tests; commit.

## Item 2 — Animate window features

From mixamo-animation-extra-features.png. Drop **Aero Update** (obsolete). Keep:

- [ ] 2a. 3-way visibility: mesh-only / skeleton-only / both. Needs an ANIMATED skeleton (SkeletonHelper-equivalent on the animated mesh, in the item-1 style), since the fitted skeleton is rest-pose only. Replaces the current "Show bones" checkbox.
- [ ] 2b. Overdrive (playback speed/intensity) slider.
- [ ] 2c. Character Arm-Space slider (retarget arm spacing).
- [ ] 2d. Trim (start/end frame range) — export/preview honor it.
- [ ] 2e. Mirror checkbox (retarget mirror).
- [ ] 2f. Verify compiled build + tests; commit.

## Item 3 — Export / Download modal

From mixamo-export-modal-options.png. A modal on Export:

- [ ] 3a. Format (glb / fbx), Skin (with / without), Frames per Second, Keyframe Reduction (none/…).
- [ ] 3b. Wire options into the export command (skin toggle, fps, keyframe reduction).
- [ ] 3c. Verify compiled build + tests; commit.

---

## Status log

- (start) Plan created. Marker fixture committed. Item-4 report delivered (defer).
- 1a DONE: bone geometry is already octahedral (Mixamo-shaped); the gap was colour.
- 1b DONE + committed (4047708): blue→violet depth gradient, MeshBasic material.
- USER FEEDBACK: "skeleton is not the one in mixamo, check closely". Fixed:
  - Long violet bone up the legs was the `root`→`pelvis` connector (root at floor
    y≈0). Skip a floor-reference root's connector in skeletonOctahedra (bottom-15%
    heuristic, safe for imported rigs whose root is a real mid-body Hips joint).
  - Giant sphere handles dominated; Mixamo bones are the primary read. Shrank
    handleRadius 1.5%→0.6% of the diagonal (small precision dots); boneWidth floor
    0.28→0.7 of a handle so bones stay prominent.
- 1c DONE: editing loupe follows a joint handle while dragged.
- **ITEM 1 DONE + committed (3b09f45)**: verified in the compiled .app — clean
  Mixamo-style skeleton (no floor connector, small handle dots, gradient bones).
  Next: ITEM 2 — Animate window. Start with 2a (3-way mesh/skeleton/both), which
  needs an animated skeleton drawn in the item-1 gradient style over the animated
  mesh (a SkeletonHelper-equivalent), replacing the "Show bones" checkbox.
