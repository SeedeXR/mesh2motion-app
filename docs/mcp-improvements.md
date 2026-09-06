# Epic: smarter, more-eyes MCP + Blender add-on

Make the rigging MCP (`crates/m2m-mcp`) and the Blender add-on
(`blender-addon/mesh2motion_bridge.py`) more robust, feature-rich, and observable.
User asked for all of: visual diagnostics, robust joint/session tools, smarter
fitting, add-on upgrade, **plus** ffmpeg animation rendering + reference-video
comparison. Standing rules: unit tests, code+ponytail+sonar review, verify, CI-green
commits with the Co-Authored-By trailer.

## Grounding (what already exists — surface, don't reinvent)
- `pipeline::BindReport`: vertices, weighted/excluded bones, **fallback_vertices**
  (disconnected islands), **influence histogram** (1..4), **unweighted_vertices**.
- `m2m_rig::fit`: `Landmarks::symmetry_error`, marker fit (`fit_from_markers`),
  `fit_uniform`. `retarget::RetargetReport`. Fit test helper `limb_joints_outside`.
- Bridge: `render_views` spawns Blender with `tools/blender-render-views.py`
  (embedded via include_str), returns PNGs. `inspect`/`inspect_live` (the add-on).
- ffmpeg at `/opt/homebrew/bin/ffmpeg`; Blender has no ffmpeg (render PNGs → encode).
- MCP tools today: session_status, list_templates, load_asset, fit_skeleton,
  adjust_joint (by index), bind_weights, list_clips, export, validate_export,
  render_views.

## Stages (priority order; commit each CI-green)
- [x] **1. `diagnose` tool (MCP).** `pipeline::diagnose` grades fit+bind pass/warn/fail:
  unweighted vertices, disconnected islands, influence histogram, joints grossly off the
  mesh (nearest-vertex distance > 8% of body size — not naive inside/outside, which flags
  every leaf/root), fit scale. Surfaced as the `diagnose` MCP tool. Also added stderr
  logging (`m2m_mcp::log`, gated by `M2M_MCP_LOG`, never on stdout) instrumenting every
  tool call with outcome + timing. Tests: unit (grade a clean fit, log format), integration
  (diagnose over the tools), regression (precondition next-step hints). fmt+clippy clean.
- [ ] **2. Visual eyes (bridge + MCP).** Extend the render script + `render_views`:
  `overlay=skeleton` (draw the fitted bones inside the mesh), `overlay=weights`
  (heatmap by summed weight / flag fallback islands). New optional arg on render_views.
- [ ] **3. Animation video + reference compare (bridge + MCP).** `render_animation`:
  render a clip's frames in Blender → encode MP4 with ffmpeg → return path (+ a frame
  strip as images). `compare_to_reference`: given a reference video path, sample frames
  from both and return a side-by-side montage (and a coarse silhouette-difference score).
- [ ] **4. Robust joint/session tools (MCP).** `list_joints` (name/pos/parent),
  `adjust_joint` by name + `nudge` delta + `mirror` L↔R, session `undo`, richer
  next-step error hints.
- [ ] **5. Smarter fitting (MCP).** `suggest_template` (rank templates for the loaded
  mesh by bone-plan/shape/axis), expose `fit_from_markers`, retarget-quality on export.
- [ ] **6. Add-on upgrade (Python).** `render` command (viewport PNG back), richer
  report (over-influence, unweighted, bbox, materials, non-manifold), N-panel status UI,
  optional socket token.

## Notes / decisions
- ffmpeg path: reuse the bridge's tool-discovery pattern (env override + PATH); if
  ffmpeg is missing, `render_animation` returns the frame strip + a clear note (degrade).
- Reference compare is a coarse visual aid (silhouette/luma diff on normalised frames),
  not a biomechanics metric — say so in the tool description.
