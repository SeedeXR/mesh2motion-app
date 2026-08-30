# Fuzz seeds

Committed starting inputs for `cargo fuzz`. `corpus/` and `artifacts/` are
generated and gitignored, so these are what CI and a fresh clone begin from.

- `ascii-minimal.fbx`, `ascii-animated.fbx` — hand-written ASCII documents
  covering geometry, a model with a `PreRotation`, and an animation curve.
  Binary FBX starts with a 23-byte magic that random mutation almost never
  reproduces, so without ASCII seeds the layers above the reader are barely
  reached.
- `ascii-skinned-quad.fbx` — a quad (so `triangulate`'s quad branch runs), a
  `ByPolygonVertex`/`Direct` normal layer and an `IndexToDirect` UV layer (so
  `Layer::at`'s mapping and indirection branches run), a `GeometricScaling`,
  and a skin `Cluster` with weights and bind matrices. Without it
  `skin::parse_all`, `Skin::bind` and the layer-element paths were reachable
  only through the binary rig, where 70% of the bytes are inside zlib streams
  and any mutation fails inflate and rejects the whole file.
- `regression-objects-child-without-numeric-id.fbx` — the input that crashed
  `fbx_pipeline` on 2026-08-30. An `Objects` child with no numeric id tripped a
  `debug_assert!` in `Scene::from_document`, i.e. a panic on file content, in
  every debug build. Fixed by counting it in `SceneReport` instead; the
  regression test is `objects_the_scene_cannot_key_are_counted_rather_than_asserted_away`
  in `crates/m2m-io/tests/fbx_dom.rs`.

The real corpus also wants `legacy/static/test-files/retarget testing/mixamo-original-rig.fbx`
and, when present locally, `references/human_based_fbx_mixamo_animations/*.fbx`.
`./seed.sh` assembles all of it.
