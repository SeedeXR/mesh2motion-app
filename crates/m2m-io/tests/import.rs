//! Format-blind import, and the O9 promise it exists to keep.
//!
//! O9 (`memory/project_context.md`): a file that arrives with a skeleton keeps
//! it. The legacy app's default was the opposite —
//! `ModelCleanupUtility.strip_out_all_unecessary_model_data` deletes
//! `skinIndex`/`skinWeight` and demotes every `SkinnedMesh` to a `Mesh`. These
//! tests pin the inversion: an already-rigged file must come back reported as
//! rigged, with its bones, its skins and its clips still counted.

use m2m_io::import::{inspect, Format, ImportError};

const MIXAMO_FBX: &[u8] =
    include_bytes!("../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx");
const RIG_ONLY: &[u8] = include_bytes!("../../../legacy/static/rigs/rig-human.glb");
const PLAIN_MESH: &[u8] = include_bytes!("../../../legacy/static/models/model-human.glb");
const ANIMATED: &[u8] =
    include_bytes!("../../../legacy/static/animations/human-base-animations.glb");
const INTERLEAVED: &[u8] =
    include_bytes!("../../../legacy/static/test-files/human-interleaved-buffer-mesh.glb");

#[test]
fn a_rigged_fbx_arrives_rigged() {
    let import = inspect(MIXAMO_FBX).expect("the reference rig reads");

    assert_eq!(import.format, Format::Fbx);
    assert!(import.already_rigged());
    assert_eq!(import.bones.len(), 65);
    assert_eq!(import.meshes, 2);
    assert_eq!(
        import.skinned_meshes, 2,
        "both meshes are bound to the skin"
    );
    assert_eq!(import.clips, ["mixamo.com", "Take 001"]);
    // Bones come back parents-first, which is the order a rebuild needs.
    assert_eq!(import.bones[0], "mixamorig:Hips");
    assert_eq!(import.bones[1], "mixamorig:Spine");
}

#[test]
fn a_plain_mesh_is_not_reported_as_rigged() {
    let import = inspect(PLAIN_MESH).expect("reads");

    // The distinction O9 turns on: this is the file that may be auto-rigged
    // without destroying anything.
    assert!(!import.already_rigged());
    assert!(import.bones.is_empty());
    assert_eq!(import.skinned_meshes, 0);
    assert_eq!(import.meshes, 1);
}

#[test]
fn a_skeleton_with_no_mesh_still_counts_as_rigged() {
    // `rig-human.glb` is a template skeleton: a skin naming 66 joints, and no
    // mesh node pointing at it. Deciding "rigged" from the skinned-mesh count
    // alone would call this file unrigged and then overwrite its skeleton.
    let import = inspect(RIG_ONLY).expect("reads");

    assert!(import.already_rigged());
    assert_eq!(import.bones.len(), 66);
    assert_eq!(import.meshes, 0);
    assert_eq!(import.skinned_meshes, 0);
}

#[test]
fn animation_is_part_of_what_survives_import() {
    let import = inspect(ANIMATED).expect("reads");

    assert_eq!(import.clips.len(), 87);
    assert_eq!(import.clips[0], "Chest_Open");
    assert_eq!(import.bones.len(), 66);
}

#[test]
fn nine_meshes_on_one_armature_are_nine_meshes_and_one_skeleton() {
    // The file has one skin of 67 joints and nine mesh nodes pointing at it.
    let import = inspect(INTERLEAVED).expect("reads");

    assert_eq!(import.meshes, 9);
    assert_eq!(import.skinned_meshes, 9);
    assert_eq!(import.bones.len(), 67);
}

#[test]
fn a_joint_two_skins_share_is_reported_once() {
    // A body and a coat exported as separate skinned meshes get a skin each,
    // both listing the same joints. Concatenating the joint lists would report
    // the skeleton twice and tell the user their model has 134 bones.
    let two_skins = rewrite_json(INTERLEAVED, |doc| {
        let copy = doc["skins"][0].clone();
        doc["skins"].as_array_mut().expect("skins").push(copy);
        let last = doc["nodes"]
            .as_array_mut()
            .expect("nodes")
            .iter_mut()
            .rfind(|n| n.get("skin").is_some())
            .expect("a skinned node");
        // Not load-bearing for the assertion — `from_glb` reads every skin the
        // file declares, referenced or not. It is here so the fixture describes
        // a file an exporter could actually write.
        last["skin"] = serde_json::json!(1);
    });

    let import = inspect(&two_skins).expect("reads");
    assert_eq!(import.bones.len(), 67);
    assert_eq!(import.skinned_meshes, 9);
}

#[test]
fn the_format_comes_from_the_bytes_not_the_name() {
    // `inspect` is never given a filename, so an extension cannot mislead it.
    assert_eq!(inspect(ANIMATED).expect("reads").format, Format::Glb);
    assert_eq!(inspect(MIXAMO_FBX).expect("reads").format, Format::Fbx);
}

#[test]
fn an_ascii_fbx_reads_as_fbx() {
    let source = concat!(
        "; FBX 7.4.0 project file\n",
        "FBXHeaderExtension:  {\n",
        "\tFBXVersion: 7400\n",
        "}\n",
        "Objects:  {\n",
        "\tModel: 1, \"Model::Hips\", \"LimbNode\" {\n",
        "\t}\n",
        "}\n",
        "Connections:  {\n",
        "}\n",
    );
    let import = inspect(source.as_bytes()).expect("ascii FBX reads");

    assert_eq!(import.format, Format::Fbx);
    assert_eq!(import.bones, ["Hips"]);
}

#[test]
fn something_that_is_not_a_model_is_refused_rather_than_guessed_at() {
    let err = inspect(b"\x89PNG\r\n\x1a\n not a model at all").expect_err("refused");
    assert!(matches!(err, ImportError::UnknownFormat), "got {err:?}");
}

#[test]
fn a_damaged_fbx_reports_the_damage_rather_than_calling_it_unknown() {
    // Truncated after the header. It still claims to be FBX, and a user whose
    // model is cut in half should not be told the format was unrecognised.
    let err = inspect(&MIXAMO_FBX[..64]).expect_err("refused");
    assert!(matches!(err, ImportError::Fbx(_)), "got {err:?}");
}

#[test]
fn extra_influence_sets_are_counted_rather_than_dropped_in_silence() {
    // glTF holds bone influences in sets of four; a vertex needing more uses
    // JOINTS_1/WEIGHTS_1 too, and this reader takes set 0 only. Whether those
    // influences are dropped is not the test — that they are *reported* is.
    let base = m2m_io::glb::read(INTERLEAVED).expect("reads");
    let skinned = base
        .primitives
        .iter()
        .filter(|p| !p.joints.is_empty())
        .count();
    assert!(skinned > 0, "the fixture must have skinned primitives");

    assert_eq!(
        inspect(INTERLEAVED).expect("reads").over_influence_limit,
        0,
        "the unmodified file is within the limit"
    );
    assert_eq!(
        inspect(&with_second_influence_set(INTERLEAVED))
            .expect("reads")
            .over_influence_limit,
        skinned
    );
}

/// Adds `JOINTS_1`/`WEIGHTS_1` to every skinned primitive of a `.glb`, pointing
/// them at the accessors set 0 already uses.
///
/// Reusing the accessors keeps the file valid — same type, same length — so
/// what changes is only the declaration that the mesh needs more than four
/// influences per vertex.
fn with_second_influence_set(glb: &[u8]) -> Vec<u8> {
    rewrite_json(glb, |doc| {
        for mesh in doc["meshes"].as_array_mut().expect("meshes") {
            for primitive in mesh["primitives"].as_array_mut().expect("primitives") {
                let attributes = primitive["attributes"].as_object_mut().expect("attributes");
                let (Some(joints), Some(weights)) = (
                    attributes.get("JOINTS_0").cloned(),
                    attributes.get("WEIGHTS_0").cloned(),
                ) else {
                    continue;
                };
                attributes.insert("JOINTS_1".into(), joints);
                attributes.insert("WEIGHTS_1".into(), weights);
            }
        }
    })
}

/// Rewrites a `.glb`'s JSON chunk, leaving its BIN chunk alone.
///
/// Editing a real file beats hand-building a minimal one: the accessors, buffer
/// views and joint data stay exactly as an exporter wrote them, so a test can
/// change one declaration and nothing else.
fn rewrite_json(glb: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let json_len = u32::from_le_bytes(glb[12..16].try_into().expect("header")) as usize;
    let mut doc: serde_json::Value =
        serde_json::from_slice(&glb[20..20 + json_len]).expect("json chunk");
    edit(&mut doc);

    // The JSON chunk is padded to a 4-byte boundary with spaces, per the spec.
    let mut json = serde_json::to_vec(&doc).expect("serialises");
    json.resize(json.len().next_multiple_of(4), b' ');

    let rest = &glb[20 + json_len..];
    let mut out = Vec::with_capacity(20 + json.len() + rest.len());
    out.extend_from_slice(&glb[..8]);
    out.extend_from_slice(&((12 + 8 + json.len() + rest.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&glb[16..20]);
    out.extend_from_slice(&json);
    out.extend_from_slice(rest);
    out
}

#[test]
fn load_brings_either_format_to_the_same_document_model() {
    // The bulk channel carries one format, whatever arrived
    // (`memory/architecture.md` §4), so `load` must land both in `glb::Document`.
    let from_fbx = m2m_io::import::load(MIXAMO_FBX).expect("fbx loads");
    assert_eq!(from_fbx.nodes.len(), 67, "65 bones and 2 mesh models");
    assert_eq!(from_fbx.skins.len(), 2);

    let from_glb = m2m_io::import::load(ANIMATED).expect("glb loads");
    assert_eq!(
        from_glb,
        m2m_io::glb::read(ANIMATED).expect("reads"),
        "a glTF is passed through, not rebuilt"
    );
}

#[test]
fn what_load_returns_can_be_written_as_the_glb_that_goes_on_the_wire() {
    let bytes =
        m2m_io::glb::write(&m2m_io::import::load(MIXAMO_FBX).expect("loads")).expect("writes");
    let back = m2m_io::glb::read(&bytes).expect("the wire payload reads back");

    assert_eq!(back.nodes.len(), 67);
    assert_eq!(back.skins.len(), 2);
    assert!(back.nodes.iter().any(|n| n.name == "mixamorig:Hips"));
}

#[test]
fn load_refuses_what_inspect_refuses() {
    assert!(matches!(
        m2m_io::import::load(b"\x89PNG\r\n\x1a\n not a model").expect_err("refused"),
        ImportError::UnknownFormat
    ));
}
