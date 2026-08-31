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
});
