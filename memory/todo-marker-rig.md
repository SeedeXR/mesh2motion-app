# TODO: marker-placement auto-rig pipeline (2026-09-04)

User request: make **marker placement** the DEFAULT auto-rig flow (Mixamo-style:
chin/wrists/elbows/knees/groin, symmetry toggle — see
references/auto-rig-mixamo-reference/*.png), keep the current automatic auto-fit
as an alternative, solve a **smooth** skeleton from the markers, then let the
user **edit bones** if the rig is off. One unified, more powerful pipeline.

Standing process per commit: reproduce/verify visually in-app, unit + integration
+ regression tests, code review + ponytail review + SonarQube review, commit/push,
CI green.

## Design (decided)

Marker = (template bone name, target world position on the mesh). Mixamo human set
→ bones:  chin→`head`, wrists→`hand_l/hand_r`, elbows→`lowerarm_l/lowerarm_r`,
knees→`calf_l/calf_r`, groin→`pelvis`. Solver is creature-agnostic (any bone list).

Solver `fit_from_markers(template, rest, parents, markers) -> Fitted`
(crates/m2m-rig/src/fit.rs), sibling to `fit_template`:
1. Uniform **scale + translation** least-squares from marked rest↔target
   correspondences (closed form, no SVD — orientation handled by the orient step,
   matching the existing pipeline's uniform-scale philosophy). <2 markers → None.
2. transformed[i] = rest[i]*s + t  (baseline placement).
3. Per-chain **delta propagation**: marked bones snap exactly to their marker;
   bones bracketed by two markers in a chain blend deltas by rest arc-length
   fraction (smooth stretch hitting both); bones before the first marker blend from
   the cross-chain parent's delta → first marker; bones after the last marker
   rigid-follow it. Chains processed first-bone-index order (parents-first rig), so
   each chain's anchor (parent-of-first-bone delta) is already resolved. Unmarked
   chains rigid-follow their parent. final[i] = transformed[i] + delta[i].
   ponytail ceiling: spine↔head interpolation is only endpoint-snap + rigid-follow
   across the spine/neck/head chain boundary (no cross-chain arc blend); global
   scale covers torso size, edit-bones covers residual. Upgrade to a cross-chain
   spine path if measured torso mismatch is visible.

## Stages
- [x] S1 (ad2c263): Rust `fit_from_markers` solver + 4 unit tests. Verified: marked
      bones land exactly, finger rigid-follows hand, intermediate interpolates,
      <2 markers None.
- [x] S2: pipeline `fit_from_markers` (extracted shared `fitted_to_skeleton` tail,
      no drift — all `fit()` tests still green) + Tauri command + IPC `fitFromMarkers`.
      `Marker.position` is `[f32;3]` (serde, crosses IPC). Pipeline integration test
      lands 6 human markers exactly. cargo+clippy+tsc clean, 36 pipeline tests pass.
- [ ] S3: frontend marker-placement UI — draggable markers, symmetry toggle,
      raycast onto mesh, project to medial depth. Default flow; current auto-fit
      as the alternative ("Auto-fit instead").
- [ ] S4: editable bones after solve (joint-drag exists) + polish; make default.

## Notes
- rest.bones order == parents index space (both from pipeline `rest_pose`).
- Marker target = joint position; frontend projects surface click to medial Z.
- Verify visually: M2M_AUTOLOAD/M2M_AUTOFIT harness + swift winid + screencapture.
