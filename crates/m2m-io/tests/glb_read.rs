//! Reading glTF 2.0 binary files.
//!
//! **Every count here was confirmed against Blender before it was written
//! down.** Blender is an independent implementation and is not in CI, so its
//! agreement is pinned here, where CI can see it. The differential sweep that
//! produced these numbers is `tools/glb-blender-diff.sh`.

use m2m_io::glb::{self, GlbError, Path};

/// Fixtures live in the legacy tree and some are several megabytes, so they are
/// read at run time rather than embedded in the test binary.
fn fixture(relative: &str) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../legacy/static/").to_owned() + relative;
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

/// A rig template: an armature and nothing else.
///
/// These nine files are what the app offers as skeletons, so reading them is
/// not optional. Blender agrees: 66 bones, no meshes.
#[test]
fn a_rig_template_is_a_skeleton_with_no_mesh() {
    let document = glb::read(&fixture("rigs/rig-human.glb")).expect("reads");
    assert_eq!(document.nodes.len(), 67);
    assert_eq!(document.skins.len(), 1);
    assert_eq!(
        document.skins[0].joints.len(),
        66,
        "Blender reports 66 bones"
    );
    assert!(
        document.primitives.is_empty(),
        "the file declares no meshes"
    );
    assert_eq!(document.clips.len(), 0);
    assert_eq!(document.report, glb::GlbReport::default());
}

/// Every joint resolves to a named node, and the hierarchy is a tree with one
/// root. A skin whose joints pointed at the wrong nodes would still have the
/// right *count*, so the names and the parent chain are what is checked.
#[test]
fn the_skeleton_resolves_to_named_bones_under_one_root() {
    let document = glb::read(&fixture("rigs/rig-human.glb")).expect("reads");
    let skin = &document.skins[0];

    let unnamed = skin
        .joints
        .iter()
        .filter(|&&j| document.nodes[j].name.is_empty())
        .count();
    assert_eq!(unnamed, 0, "every joint should be a named bone");

    // The root bone is not a root *node*: it hangs off an "Armature" container
    // that is part of the scene graph but not a joint. So the root bone is the
    // joint whose parent is not itself a joint, and a reader that looked for a
    // parentless joint would find none.
    let joints: std::collections::HashSet<usize> = skin.joints.iter().copied().collect();
    let roots: Vec<&str> = skin
        .joints
        .iter()
        .filter(|&&j| {
            !document.nodes[j]
                .parent
                .is_some_and(|p| joints.contains(&p))
        })
        .map(|&j| document.nodes[j].name.as_str())
        .collect();
    assert_eq!(roots, vec!["root"], "one root bone");
    let root = skin
        .joints
        .iter()
        .find(|&&j| document.nodes[j].name == "root");
    let container = document.nodes[*root.expect("root bone")]
        .parent
        .expect("armature");
    assert_eq!(document.nodes[container].name, "Armature");
    assert_eq!(
        document.nodes[container].parent, None,
        "the armature is a scene root"
    );

    assert!(document.nodes.iter().any(|n| n.name == "head"));
    assert_eq!(
        skin.inverse_bind_matrices.len(),
        skin.joints.len(),
        "one inverse bind matrix per joint"
    );
}

/// Interleaved vertex attributes: position, joints and weights sharing one
/// buffer view with a stride. Reading these wrong yields plausible-looking
/// garbage rather than an error, so the counts are pinned against Blender's.
#[test]
fn interleaved_attributes_read_the_same_as_blender_reads_them() {
    let document =
        glb::read(&fixture("test-files/human-interleaved-buffer-mesh.glb")).expect("reads");

    let vertices: usize = document
        .primitives
        .iter()
        .map(|p| p.positions.len() / 3)
        .sum();
    let triangles: usize = document
        .primitives
        .iter()
        .map(|p| p.indices.len() / 3)
        .sum();
    let weighted: usize = document
        .primitives
        .iter()
        .map(|p| {
            p.weights
                .chunks_exact(4)
                .filter(|w| w.iter().any(|&x| x > 0.0))
                .count()
        })
        .sum();

    assert_eq!(document.mesh_count(), 9);
    assert_eq!(vertices, 11_702);
    assert_eq!(triangles, 19_914);
    assert_eq!(document.skins[0].joints.len(), 67);
    assert_eq!(weighted, 11_702, "every vertex is weighted");

    // Four influences per vertex, matching the joint array.
    for primitive in &document.primitives {
        assert_eq!(primitive.joints.len(), primitive.weights.len());
        assert_eq!(primitive.weights.len() % 4, 0);
        assert_eq!(primitive.joints.len() / 4, primitive.positions.len() / 3);
    }
}

/// A glTF mesh holds one primitive per material, and importers merge them into
/// one object. `human-jay.glb` is a single mesh of 22 primitives, and Blender
/// imports it as one mesh — so counting primitives as meshes is wrong by 21.
///
/// This cost real time to find: four files "disagreed" with Blender until the
/// comparison stopped equating the two.
#[test]
fn many_primitives_can_belong_to_one_mesh() {
    let document = glb::read(&fixture("models-variation/human-jay.glb")).expect("reads");
    assert_eq!(document.primitives.len(), 22);
    assert_eq!(document.mesh_count(), 1, "Blender imports this as one mesh");
    assert!(document.primitives.iter().all(|p| p.mesh == 0));
}

/// Animation, checked the way the FBX writer taught: not just how many keys,
/// but *when* they are.
///
/// Blender reports `Chest_Open` as 660 curves, 5,128 keys, frames 0-33. Those
/// relate to glTF's own numbers exactly: 66 bones x (3 translation + 4
/// quaternion + 3 scale) = 660 curves per 198 channels, the same keys counted
/// per component rather than per channel, and 1.375 s x 24 fps = 33 frames.
#[test]
fn animation_matches_blender_on_channels_keys_and_time() {
    let document = glb::read(&fixture("animations/human-base-animations.glb")).expect("reads");
    assert_eq!(document.clips.len(), 87);

    let clip = document
        .clips
        .iter()
        .find(|c| c.name == "Chest_Open")
        .expect("Chest_Open");
    assert_eq!(clip.channels.len(), 198, "66 bones x 3 paths");

    let keys: usize = clip.channels.iter().map(|c| c.times.len()).sum();
    assert_eq!(keys, 1_356);

    // Blender splits a channel into one F-curve per component, so its key total
    // is the stride-weighted one.
    let component_keys: usize = clip
        .channels
        .iter()
        .map(|c| c.times.len() * c.path.stride().unwrap_or(1))
        .sum();
    assert_eq!(component_keys, 5_128, "Blender's key count for this clip");

    // The time axis. A clip with every count right and the duration wrong plays
    // at the wrong speed, which is exactly how the FBX writer's missing
    // TimeMode hid: 1.375 s is 33 frames at Blender's 24 fps.
    assert!(
        (clip.duration - 1.375).abs() < 1e-6,
        "duration was {}",
        clip.duration
    );

    // Values are stride x keys, so a truncated accessor cannot pass.
    for channel in &clip.channels {
        if let Some(stride) = channel.path.stride() {
            assert_eq!(
                channel.values.len(),
                channel.times.len() * stride,
                "channel on node {} has {} values for {} keys",
                channel.node,
                channel.values.len(),
                channel.times.len()
            );
        }
        assert!(matches!(
            channel.path,
            Path::Translation | Path::Rotation | Path::Scale
        ));
    }
}

/// A `.gltf` whose buffer points at another file is refused, not fetched.
///
/// This is the trust boundary: honouring the URI would let an opened model
/// choose what to read off the disk or the network.
#[test]
fn a_buffer_pointing_outside_the_file_is_refused() {
    let json = br#"{
        "asset": {"version": "2.0"},
        "buffers": [{"uri": "../../../etc/passwd", "byteLength": 4}]
    }"#;
    match glb::read(json) {
        Err(GlbError::ExternalBuffer { index: 0 }) => {}
        other => panic!("expected the external buffer to be refused, got {other:?}"),
    }
}

/// A data: URI is still external data chosen by the file, and is refused for
/// the same reason — the reader resolves the BIN chunk and nothing else.
#[test]
fn an_embedded_data_uri_buffer_is_also_refused() {
    let json = br#"{
        "asset": {"version": "2.0"},
        "buffers": [{"uri": "data:application/octet-stream;base64,AAAAAA==", "byteLength": 4}]
    }"#;
    assert!(matches!(
        glb::read(json),
        Err(GlbError::ExternalBuffer { index: 0 })
    ));
}

/// Malformed input errors rather than panicking. The fuzz target covers this
/// far more broadly; these are the shapes worth naming.
#[test]
fn malformed_input_errors_rather_than_panicking() {
    for (name, bytes) in [
        ("empty", &b""[..]),
        ("not glb", &b"hello"[..]),
        ("magic only", &b"glTF"[..]),
        ("truncated header", &b"glTF\x02\x00\x00\x00"[..]),
        (
            "header, no chunks",
            &b"glTF\x02\x00\x00\x00\x0c\x00\x00\x00"[..],
        ),
        ("empty json object", &b"{}"[..]),
    ] {
        assert!(glb::read(bytes).is_err(), "{name} should not read");
    }
}

/// Truncating a real file at many offsets must never panic. The file is valid
/// up to the cut, so the parser is being asked to handle a plausible-looking
/// prefix rather than obvious noise.
#[test]
fn truncated_files_never_panic() {
    let bytes = fixture("rigs/rig-human.glb");
    for cut in (0..bytes.len()).step_by(97) {
        let _ = glb::read(&bytes[..cut]);
    }
}

// ---------------------------------------------------------------------------
// Regressions from fuzzing. Each of these crashed the reader or a crate beneath
// it on input a file can choose, so each is pinned here.
// ---------------------------------------------------------------------------

/// Assembles a GLB from a JSON chunk and a BIN chunk, so a test can say exactly
/// what the file claims. Both chunks are padded to four bytes, as the spec
/// requires.
fn build_glb(json: &str, bin: &[u8]) -> Vec<u8> {
    fn chunk(kind: u32, payload: &[u8], pad: u8) -> Vec<u8> {
        let padding = (4 - payload.len() % 4) % 4;
        let length = payload.len() + padding;
        let mut out = Vec::with_capacity(8 + length);
        out.extend_from_slice(&(length as u32).to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(payload);
        out.extend(std::iter::repeat_n(pad, padding));
        out
    }
    let mut body = chunk(0x4E4F_534A, json.as_bytes(), b' ');
    if !bin.is_empty() {
        body.extend(chunk(0x004E_4942, bin, 0));
    }
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// A triangle naming a vertex the mesh does not have is dropped, not returned.
///
/// glTF validation checks that an accessor fits in its buffer, not that the
/// values inside an index accessor are vertices that exist — so this file is
/// "valid". Callers treat `indices` as offsets into `positions`, and a renderer
/// that believed this one would read out of bounds.
#[test]
fn a_triangle_naming_a_missing_vertex_is_dropped() {
    // Three vertices, then six indices: one good triangle and one naming vertex
    // 99. 36 bytes of positions, then 12 bytes of u16 indices.
    let mut bin = Vec::new();
    for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    for index in [0u16, 1, 2, 0, 1, 99] {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    let json = r#"{
      "asset": {"version": "2.0"},
      "buffers": [{"byteLength": 48}],
      "bufferViews": [
        {"buffer": 0, "byteOffset": 0,  "byteLength": 36},
        {"buffer": 0, "byteOffset": 36, "byteLength": 12}
      ],
      "accessors": [
        {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
         "min": [0,0,0], "max": [1,1,0]},
        {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}
      ],
      "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
      "nodes": [{"mesh": 0}],
      "scenes": [{"nodes": [0]}],
      "scene": 0
    }"#;
    let document = glb::read(&build_glb(json, &bin)).expect("reads");

    assert_eq!(document.primitives.len(), 1);
    let primitive = &document.primitives[0];
    assert_eq!(primitive.positions.len() / 3, 3);
    assert_eq!(
        primitive.indices,
        vec![0, 1, 2],
        "the triangle naming vertex 99 should be gone"
    );
    assert_eq!(document.report.out_of_range_triangles, 1);

    // The invariant every caller relies on.
    let vertices = primitive.positions.len() / 3;
    assert!(primitive.indices.iter().all(|&i| (i as usize) < vertices));
}

/// A declared length below the 12-byte header does not panic.
///
/// `gltf-1.4.1/src/binary.rs:252` computes `header.length - 12` on this value,
/// which underflows. Release wraps it; debug panics, and `cargo test` is debug.
#[test]
fn a_glb_length_shorter_than_its_header_is_rejected() {
    let mut bytes = b"glTF".to_vec();
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes()); // total length of 3
    bytes.extend_from_slice(&[0; 16]);
    assert!(matches!(
        glb::read(&bytes),
        Err(GlbError::MalformedHeader { .. })
    ));
}

/// An index pointing past the array it indexes is rejected before the `gltf`
/// crate dereferences it.
///
/// Its own validator does `root.accessors[index]`
/// (`gltf-json-1.4.1/src/mesh.rs:151`) *before* checking the index, so an
/// out-of-range POSITION panics the validator instead of being reported by it —
/// in release as well as debug.
#[test]
fn an_index_past_the_end_of_its_array_is_rejected() {
    let json = r#"{
      "asset": {"version": "2.0"},
      "accessors": [],
      "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}]
    }"#;
    match glb::read(&build_glb(json, &[])) {
        Err(GlbError::IndexOutOfRange {
            owner: "primitive.attributes",
            index: 0,
            len: 0,
            ..
        }) => {}
        other => panic!("expected the dangling accessor to be rejected, got {other:?}"),
    }
}

/// A joint index past the end of the node list is rejected too — the same
/// class, reached through the skin rather than the mesh.
#[test]
fn a_joint_index_past_the_end_of_the_nodes_is_rejected() {
    let json = r#"{
      "asset": {"version": "2.0"},
      "nodes": [{"name": "only"}],
      "skins": [{"joints": [0, 7]}]
    }"#;
    assert!(matches!(
        glb::read(&build_glb(json, &[])),
        Err(GlbError::IndexOutOfRange {
            owner: "skin.joints",
            index: 7,
            len: 1,
            ..
        })
    ));
}

/// An accessor declaring a type glTF does not allow for its use is skipped.
///
/// The crate reads an accessor into a fixed Rust type and `debug_assert!`s the
/// sizes agree (`gltf-1.4.1/src/accessor/util.rs:371`), so POSITION declared as
/// anything but VEC3/f32 panics it in a debug build.
#[test]
fn an_accessor_of_the_wrong_type_is_skipped_not_read() {
    // POSITION with the right dimensions but unsigned-byte components rather
    // than f32. The shape is what the crate's validator checks, so this passes
    // validation and reaches the reader — which is the point.
    let json = r#"{
      "asset": {"version": "2.0"},
      "buffers": [{"byteLength": 12}],
      "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 9}],
      "accessors": [
        {"bufferView": 0, "componentType": 5121, "count": 3, "type": "VEC3",
         "min": [0, 0, 0], "max": [1, 1, 1]}
      ],
      "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
      "nodes": [{"mesh": 0}],
      "scenes": [{"nodes": [0]}],
      "scene": 0
    }"#;
    let document = glb::read(&build_glb(json, &[0; 12])).expect("reads");
    assert!(document.primitives.is_empty());
    assert_eq!(document.report.invalid_accessors, 1);
}

/// A skin's joint list is not the set of bones that deform the mesh.
///
/// Found while reading reference animals a user supplied: an `african
/// buffalo.glb` carries four `PoleTarget` bones that drive an IK solve and are
/// weighted to nothing, sitting outside the body by design. Our own rigs have
/// the same shape for a different reason — a root and two fingertip markers.
///
/// It matters because anything asking "is this bone inside the mesh" or "should
/// this bone be exported" gets the wrong answer for these.
#[test]
fn joints_that_deform_nothing_are_reported() {
    let document = glb::read(&fixture("models-variation/human-sophia.glb")).expect("reads");
    let skin = &document.skins[0];
    let idle = document.non_deforming_joints(0);
    let names: Vec<&str> = idle
        .iter()
        .map(|&i| document.nodes[skin.joints[i]].name.as_str())
        .collect();

    assert_eq!(
        names,
        vec!["root", "thumb_04_leaf_l", "thumb_04_leaf_r"],
        "a root and two fingertip markers carry no weight"
    );
    assert_eq!(skin.joints.len() - idle.len(), 63, "63 of 66 bones deform");
}

/// A skin index that does not exist yields nothing rather than panicking.
#[test]
fn non_deforming_joints_of_a_missing_skin_is_empty() {
    let document = glb::read(&fixture("models/model-human.glb")).expect("reads");
    assert!(document.skins.is_empty(), "this model has no skin");
    assert!(document.non_deforming_joints(0).is_empty());
    assert!(document.non_deforming_joints(99).is_empty());
}
