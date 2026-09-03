#![no_main]
//! The glTF/GLB reader on arbitrary bytes.
//!
//! The contract is `memory/test.md` §4: a malformed file returns an error. It
//! never panics, never hangs, never exhausts memory.
//!
//! This drives the whole read, not just the container parse — accessors, the
//! skin, and the animation channels included. Both trust-boundary defects found
//! by hand in the FBX work were in layers above the parser, not in the parser,
//! which is why `fbx_pipeline` exists and why this target goes all the way
//! through rather than stopping at `Gltf::from_slice`.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(document) = m2m_io::glb::read(data) else {
        return;
    };
    // Touch what the reader produced. A length or stride the file controls
    // could otherwise be wrong without anything reading it here.
    for primitive in &document.primitives {
        let vertices = primitive.positions.len() / 3;
        for &index in &primitive.indices {
            assert!(
                (index as usize) < vertices || vertices == 0,
                "index {index} out of range for {vertices} vertices"
            );
        }
        assert!(primitive.mesh < document.mesh_count().max(1));
    }
    for skin in &document.skins {
        for &joint in &skin.joints {
            assert!(joint < document.nodes.len(), "joint index out of range");
        }
    }
    for clip in &document.clips {
        for channel in &clip.channels {
            assert!(channel.node < document.nodes.len(), "channel node out of range");
        }
    }

    // Then write it back. A document assembled from hostile bytes is still a
    // document the writer may be handed — the app re-exports what it opened —
    // so the write path is fuzzed through the read path rather than separately.
    let Ok(bytes) = m2m_io::glb::write(&document) else {
        return;
    };
    // What we write, we must be able to read: an exporter that emits files its
    // own importer rejects is broken even when nothing panics. This caught a
    // real one — an empty scene serialized to `{}` and then failed to
    // deserialize, because `gltf-json`'s `Scene::nodes` has
    // `skip_serializing_if` and no `serde(default)`.
    let again = m2m_io::glb::write(&document).expect("writing twice must agree");
    assert_eq!(bytes, again, "writing is not deterministic");
    match m2m_io::glb::read(&bytes) {
        Ok(reread) => {
            assert_eq!(
                reread.nodes.len(),
                document.nodes.len(),
                "the round trip changed the node count"
            );
            // Primitives that draw nothing are not written — see the writer's
            // note on zero-count accessors — so the invariant is about the ones
            // that do.
            let drawable = document
                .primitives
                .iter()
                .filter(|p| !p.indices.is_empty() && !p.positions.is_empty())
                .count();
            assert_eq!(
                reread.primitives.len(),
                drawable,
                "the round trip changed the drawable primitive count"
            );
            assert_eq!(
                reread.clips.len(),
                document.clips.len(),
                "the round trip changed the clip count"
            );
        }
        Err(error) => panic!("wrote a file we cannot read: {error}"),
    }
});
