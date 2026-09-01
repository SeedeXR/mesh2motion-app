//! Writing glTF 2.0 binary.
//!
//! The acceptance gate is Blender: a rewritten file must produce the *same*
//! import report as the file it came from. That was confirmed for the rig
//! template, the interleaved skinned mesh, the 22-primitive mesh and the
//! 87-clip animation pack — every field identical, bone rest positions and all
//! 87 frame ranges included. Blender is not in CI, so the same properties are
//! asserted here through our own reader, plus the ones only a reader can see.
//!
//! Our own reader agreeing is weak evidence on its own — that is the trap
//! session 023 named. These tests earn their keep by checking *properties* a
//! shared bug would still break: exact float equality of keys and transforms,
//! the hierarchy, and the invariants a consumer relies on.

use m2m_io::glb::{self, Document};

fn fixture(relative: &str) -> Vec<u8> {
    // Rig `.glb` files moved to `assets/rigs/` (P3-3d); other fixtures stay in legacy.
    let path = match relative.strip_prefix("rigs/") {
        Some(rig) => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs/").to_owned() + rig,
        None => concat!(env!("CARGO_MANIFEST_DIR"), "/../../legacy/static/").to_owned() + relative,
    };
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn round_trip(relative: &str) -> (Document, Document) {
    let before = glb::read(&fixture(relative)).expect("reads the fixture");
    let bytes = glb::write(&before).expect("writes");
    let after = glb::read(&bytes).expect("reads back what it wrote");
    (before, after)
}

fn vertices(document: &Document) -> usize {
    document
        .primitives
        .iter()
        .map(|p| p.positions.len() / 3)
        .sum()
}

fn triangles(document: &Document) -> usize {
    document
        .primitives
        .iter()
        .map(|p| p.indices.len() / 3)
        .sum()
}

/// A skeleton-only template survives, including the fact that it has no mesh.
///
/// Writing a stray empty mesh here would be invisible in a vertex count and
/// obvious in a DCC's outliner.
#[test]
fn a_rig_template_round_trips() {
    let (before, after) = round_trip("rigs/rig-human.glb");
    assert_eq!(after.nodes.len(), before.nodes.len());
    assert_eq!(after.skins.len(), 1);
    assert_eq!(after.skins[0].joints, before.skins[0].joints);
    assert!(after.primitives.is_empty(), "no mesh should be invented");
    assert_eq!(after.report, glb::GlbReport::default());
}

/// Geometry and skinning survive exactly, and the mesh grouping with them.
///
/// Blender confirmed this file rewrites to an identical report: 9 meshes,
/// 67 bones, 11,702 vertices, 19,914 polygons, 11,702 weighted, 67 groups.
#[test]
fn a_skinned_mesh_round_trips_exactly() {
    let (before, after) = round_trip("test-files/human-interleaved-buffer-mesh.glb");

    assert_eq!(after.mesh_count(), before.mesh_count());
    assert_eq!(after.primitives.len(), before.primitives.len());
    assert_eq!(vertices(&after), vertices(&before));
    assert_eq!(triangles(&after), triangles(&before));
    assert_eq!(after.report, glb::GlbReport::default());

    // Positions and indices are compared value by value, not by count: a
    // writer that reordered or rescaled them would keep every count intact.
    for (a, b) in after.primitives.iter().zip(&before.primitives) {
        assert_eq!(a.positions, b.positions, "positions changed");
        assert_eq!(a.indices, b.indices, "indices changed");
        assert_eq!(a.joints, b.joints, "joint indices changed");
        assert_eq!(a.weights, b.weights, "weights changed");
        assert_eq!(a.mesh, b.mesh, "primitive moved to another mesh");
        assert_eq!(a.node, b.node, "primitive moved to another node");
    }
}

/// The skin stays attached to the node that carries the mesh.
///
/// glTF puts the skin on the node, not the mesh. Losing that writes an armature
/// and a mesh with no link between them: the file still has 67 bones and every
/// weight, and imports completely unweighted.
#[test]
fn the_skin_stays_attached_to_its_node() {
    let (before, after) = round_trip("test-files/human-interleaved-buffer-mesh.glb");
    let skinned = |d: &Document| -> Vec<(usize, Option<usize>)> {
        d.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.skin.is_some())
            .map(|(i, n)| (i, n.skin))
            .collect()
    };
    assert_eq!(skinned(&after), skinned(&before));
    assert!(
        !skinned(&after).is_empty(),
        "this fixture is skinned, so something must carry the skin"
    );
}

/// The bone hierarchy and names survive.
///
/// Session 023 learned this the hard way on the FBX side: comparing bone
/// *names* alone let a flattened hierarchy pass, because the names were all
/// still there.
#[test]
fn the_bone_hierarchy_survives() {
    let (before, after) = round_trip("rigs/rig-human.glb");
    let chain = |d: &Document| -> Vec<(String, Option<String>)> {
        d.nodes
            .iter()
            .map(|n| (n.name.clone(), n.parent.map(|p| d.nodes[p].name.clone())))
            .collect()
    };
    assert_eq!(chain(&after), chain(&before), "the parent chain changed");
    assert_eq!(
        after.nodes.iter().filter(|n| n.parent.is_none()).count(),
        before.nodes.iter().filter(|n| n.parent.is_none()).count(),
        "the number of scene roots changed"
    );
}

/// Rest transforms survive bit for bit.
///
/// Kept as TRS rather than a matrix precisely so this holds: composing to a
/// matrix and decomposing back would drift, and a drifting bind pose is the
/// kind of error that shows up as a slightly wrong mesh nobody can attribute.
#[test]
fn bone_transforms_survive_exactly() {
    let (before, after) = round_trip("rigs/rig-human.glb");
    for (a, b) in after.nodes.iter().zip(&before.nodes) {
        assert_eq!(a.transform.translation, b.transform.translation);
        assert_eq!(a.transform.rotation, b.transform.rotation);
        assert_eq!(a.transform.scale, b.transform.scale);
    }
    assert_eq!(
        after.skins[0].inverse_bind_matrices, before.skins[0].inverse_bind_matrices,
        "the bind pose changed"
    );
}

/// Animation survives, and so does its time axis.
///
/// Session 024's lesson, applied: a clip with the right channel count, the right
/// key count and the wrong times plays at the wrong speed. Blender reads a
/// clip's range from the input accessor's min/max, so those are asserted too —
/// omitting them is exactly how the FBX writer's missing TimeMode hid.
#[test]
fn animation_and_its_time_axis_survive() {
    let (before, after) = round_trip("animations/human-base-animations.glb");
    assert_eq!(after.clips.len(), 87);
    assert_eq!(after.clips.len(), before.clips.len());

    for (a, b) in after.clips.iter().zip(&before.clips) {
        assert_eq!(a.name, b.name, "clip renamed");
        assert_eq!(
            a.channels.len(),
            b.channels.len(),
            "{} lost channels",
            a.name
        );
        assert_eq!(a.duration, b.duration, "{} changed length", a.name);
        for (x, y) in a.channels.iter().zip(&b.channels) {
            assert_eq!(x.node, y.node, "{} channel retargeted", a.name);
            assert_eq!(x.path, y.path, "{} channel changed property", a.name);
            assert_eq!(x.times, y.times, "{} key times changed", a.name);
            assert_eq!(x.values, y.values, "{} key values changed", a.name);
        }
    }
}

/// Every key-time accessor carries min and max, which is where an importer
/// reads the clip's range from.
#[test]
fn key_time_accessors_declare_their_range() {
    let before = glb::read(&fixture("animations/human-base-animations.glb")).expect("reads");
    let bytes = glb::write(&before).expect("writes");
    let json = json_of(&bytes);

    let accessors = json["accessors"].as_array().expect("accessors");
    let samplers: Vec<&serde_json::Value> = json["animations"]
        .as_array()
        .expect("animations")
        .iter()
        .flat_map(|a| a["samplers"].as_array().expect("samplers"))
        .collect();
    assert!(!samplers.is_empty());
    for sampler in samplers {
        let input = sampler["input"].as_u64().expect("input") as usize;
        let accessor = &accessors[input];
        assert!(
            accessor["min"].is_array() && accessor["max"].is_array(),
            "a key-time accessor with no min/max leaves the clip range to be guessed"
        );
    }
}

/// POSITION carries min and max, which glTF requires and the `gltf` crate's own
/// validator enforces — a file without them fails to load at all.
#[test]
fn position_accessors_declare_their_bounds() {
    let before = glb::read(&fixture("models-variation/human-sophia.glb")).expect("reads");
    let bytes = glb::write(&before).expect("writes");
    let json = json_of(&bytes);
    let accessors = json["accessors"].as_array().expect("accessors");

    let mut checked = 0;
    for mesh in json["meshes"].as_array().expect("meshes") {
        for primitive in mesh["primitives"].as_array().expect("primitives") {
            let index = primitive["attributes"]["POSITION"]
                .as_u64()
                .expect("POSITION") as usize;
            let accessor = &accessors[index];
            let min = accessor["min"].as_array().expect("min");
            let max = accessor["max"].as_array().expect("max");
            assert_eq!(min.len(), 3);
            assert_eq!(max.len(), 3);
            for axis in 0..3 {
                let (lo, hi) = (
                    min[axis].as_f64().expect("min value"),
                    max[axis].as_f64().expect("max value"),
                );
                assert!(lo <= hi, "bounds inverted on axis {axis}: {lo} > {hi}");
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "this fixture should have primitives");
}

/// Nothing written points outside the file.
///
/// The reader refuses external buffers; the writer must never produce one, or
/// this app becomes the thing that emits files other tools cannot open on their
/// own. One buffer, no URI, all data in the BIN chunk.
#[test]
fn the_written_file_references_nothing_outside_itself() {
    let before =
        glb::read(&fixture("test-files/human-interleaved-buffer-mesh.glb")).expect("reads");
    let bytes = glb::write(&before).expect("writes");
    let json = json_of(&bytes);

    let buffers = json["buffers"].as_array().expect("buffers");
    assert_eq!(buffers.len(), 1, "everything belongs in the one BIN chunk");
    assert!(
        buffers[0].get("uri").is_none_or(serde_json::Value::is_null),
        "the buffer must have no URI"
    );
    assert!(json
        .get("images")
        .is_none_or(|i| i.as_array().is_none_or(std::vec::Vec::is_empty)));
    assert!(
        !String::from_utf8_lossy(&bytes).contains("\"uri\""),
        "no URI of any kind should be written"
    );
}

/// Writing is deterministic: the same document twice gives the same bytes.
///
/// Not a cosmetic property. Without it, a diff of two exports is noise, and
/// nothing downstream can be content-addressed or cached.
#[test]
fn writing_the_same_document_twice_gives_the_same_bytes() {
    let document = glb::read(&fixture("rigs/rig-fox.glb")).expect("reads");
    let once = glb::write(&document).expect("writes");
    let twice = glb::write(&document).expect("writes");
    assert_eq!(once, twice);
}

/// A document written from nothing is still a loadable file.
#[test]
fn an_empty_document_writes_a_valid_file() {
    let document = Document {
        nodes: Vec::new(),
        primitives: Vec::new(),
        skins: Vec::new(),
        clips: Vec::new(),
        report: glb::GlbReport::default(),
    };
    let bytes = glb::write(&document).expect("writes");
    let back = glb::read(&bytes).expect("reads back");
    assert!(back.nodes.is_empty());
    assert!(back.primitives.is_empty());
}

/// The JSON chunk of a written GLB, for tests that need to inspect the
/// structure rather than what our reader makes of it.
fn json_of(bytes: &[u8]) -> serde_json::Value {
    assert_eq!(&bytes[..4], b"glTF");
    let length = u32::from_le_bytes(bytes[12..16].try_into().expect("chunk length")) as usize;
    let kind = &bytes[16..20];
    assert_eq!(kind, b"JSON", "the JSON chunk must come first");
    serde_json::from_slice(&bytes[20..20 + length]).expect("valid JSON chunk")
}

/// A mesh of many primitives is written back as one mesh, not many.
///
/// **Found by mutation**: making every primitive its own mesh passed every other
/// test in this file, because the skinned fixture happens to have exactly one
/// primitive per mesh — so 9 primitives and 9 meshes are the same number and
/// the grouping was never actually checked. `human-jay.glb` is one mesh of 22
/// primitives, which is the only shape that can tell the difference.
///
/// It is not cosmetic: Blender imports one glTF mesh as one object, so getting
/// this wrong turns one character into 22 separate objects in the outliner.
#[test]
fn a_mesh_of_many_primitives_is_written_as_one_mesh() {
    let (before, after) = round_trip("models-variation/human-jay.glb");
    assert_eq!(before.primitives.len(), 22, "fixture changed");
    assert_eq!(before.mesh_count(), 1, "fixture changed");

    assert_eq!(after.primitives.len(), 22);
    assert_eq!(
        after.mesh_count(),
        1,
        "the primitives were split into meshes"
    );
    assert!(
        after.primitives.iter().all(|p| p.mesh == 0),
        "every primitive belongs to the one mesh"
    );

    // And the JSON says so, independently of what our reader makes of it.
    let json = json_of(&glb::write(&before).expect("writes"));
    let meshes = json["meshes"].as_array().expect("meshes");
    assert_eq!(meshes.len(), 1);
    assert_eq!(
        meshes[0]["primitives"]
            .as_array()
            .expect("primitives")
            .len(),
        22
    );
}
