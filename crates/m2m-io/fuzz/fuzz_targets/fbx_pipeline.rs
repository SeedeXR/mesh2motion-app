#![no_main]
//! Everything the readers feed: the DOM and every layer built on it.
//!
//! This is the target that matters most. Both trust-boundary defects found by
//! hand in sessions 017 and 018 were in LAYERS, not in the readers — a
//! singular matrix inverting to NaN, and a subdivision count taken from a
//! degree delta that could saturate to `usize::MAX` and never terminate.
//! Fuzzing only `binary::parse` would have found neither.

use libfuzzer_sys::fuzz_target;
use m2m_io::fbx::{animation, binary, dom::Scene, geometry, model, skin, text};

fuzz_target!(|data: &[u8]| {
    // Accept whichever reader the bytes suit, so the corpus can hold both
    // formats and mutations of either reach the layers below.
    let Ok(document) = binary::parse(data).or_else(|_| match std::str::from_utf8(data) {
        Ok(t) => text::parse(t),
        Err(_) => Err(m2m_io::fbx::FbxError::Malformed {
            what: "input",
            detail: "not utf-8".into(),
        }),
    }) else {
        return;
    };

    let scene = Scene::from_document(document);

    // Geometry first: the skin layer needs a mesh to bind onto.
    let mut meshes = Vec::new();
    for object in scene.objects_of_kind("Geometry") {
        // The transform must come from the FILE. Passing the default made
        // `identity` always true in `geometry::parse`, so the pre-transform
        // branch — an inverse-transpose that is NaN for a zero
        // GeometricScaling — was never fuzzed, and neither was
        // `for_geometry`, which is entirely file-driven.
        let pre = geometry::GeometricTransform::for_geometry(&scene, object.id);
        if let Ok(mesh) = geometry::parse(object, pre) {
            meshes.push(mesh);
        }
    }

    let models = model::parse_all(&scene);
    // `ancestors` walks parent links; a cycle in the connection graph must not
    // make it loop.
    for m in &models.models {
        let _ = models.ancestors(m.id);
    }

    let (skins, _skipped) = skin::parse_all(&scene);
    for skin in &skins {
        let _ = skin.bone_ids();
        for cluster in &skin.clusters {
            let _ = cluster.inverse_bind();
        }
        // Bind against every mesh, not just the matching one: the mismatch
        // path is itself a trust boundary and must error rather than panic.
        for mesh in &meshes {
            let _ = skin.bind(mesh);
        }
    }

    let (clips, _report) = animation::parse_all(&scene, &models);
    for clip in &clips {
        for track in &clip.tracks {
            // The invariant every consumer relies on. A track whose values do
            // not match its stride would index out of bounds downstream, so
            // assert it here rather than waiting for a renderer to crash.
            assert_eq!(
                track.values.len(),
                track.times.len() * track.kind.stride(),
                "track stride broken"
            );
        }
    }
});
