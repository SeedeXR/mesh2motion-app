//! End-to-end pose verification (P3-P3): a T-pose clip retargeted onto both a
//! T-pose and an A-pose human must animate without the arms flying off.
//!
//! This runs the whole app pipeline — fit (with the reoriented rest rotations),
//! bind, retarget, export — and then evaluates the exported clip through the
//! same GLB reader the viewport uses, composing the bone hierarchy at samples
//! across the clip. If the rest rotations disagreed with the fitted positions,
//! the arms would swing around the wrong axis and leave the character's box;
//! this asserts they stay inside it.

use glam::{Mat4, Quat, Vec3};
use mesh2motion_lib::rig;

fn asset(rel: &str) -> Vec<u8> {
    std::fs::read(format!("../{rel}")).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}

/// The animated `.glb` for `template` fitted onto `mesh_path`, with `clip` from
/// `library` retargeted onto it.
fn animated_export(template: &str, mesh_path: &str, library_path: &str, clip: &str) -> Vec<u8> {
    let model = asset(mesh_path);
    let skeleton = rig::fit(template, &model).expect("fits");
    let library = asset(library_path);
    rig::export_glb(&model, &skeleton, 2.0, Some((&library, clip))).expect("exports")
}

/// The first clip a library offers, for tests that only need "some real motion".
fn first_clip(library_path: &str) -> String {
    let library = asset(library_path);
    rig::library_clips(&library)
        .expect("reads the library")
        .first()
        .expect("a clip")
        .name
        .clone()
}

/// Every bone's world position at time `t`, from the rest pose with the clip's
/// channels applied. Uses nearest-key sampling — the retargeter writes dense
/// keys, so it is faithful enough to catch a bone leaving the character's box.
fn bone_positions_at(
    document: &m2m_io::glb::Document,
    clip: &m2m_io::glb::Clip,
    t: f32,
) -> Vec<Vec3> {
    let mut translation: Vec<Vec3> = document
        .nodes
        .iter()
        .map(|n| Vec3::from(n.transform.translation))
        .collect();
    let mut rotation: Vec<Quat> = document
        .nodes
        .iter()
        .map(|n| Quat::from_array(n.transform.rotation))
        .collect();

    for channel in &clip.channels {
        let Some(key) = nearest_key(&channel.times, t) else {
            continue;
        };
        match channel.path {
            m2m_io::glb::Path::Translation => {
                let v = &channel.values[key * 3..key * 3 + 3];
                translation[channel.node] = Vec3::new(v[0], v[1], v[2]);
            }
            m2m_io::glb::Path::Rotation => {
                let v = &channel.values[key * 4..key * 4 + 4];
                rotation[channel.node] = Quat::from_xyzw(v[0], v[1], v[2], v[3]);
            }
            _ => {}
        }
    }

    let mut world = vec![Mat4::IDENTITY; document.nodes.len()];
    for node in 0..document.nodes.len() {
        let local = Mat4::from_rotation_translation(rotation[node].normalize(), translation[node]);
        world[node] = match document.nodes[node].parent {
            Some(parent) => world[parent] * local,
            None => local,
        };
    }
    world
        .iter()
        .map(|m| m.transform_point3(Vec3::ZERO))
        .collect()
}

fn nearest_key(times: &[f32], t: f32) -> Option<usize> {
    times
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - t).abs().total_cmp(&(**b - t).abs()))
        .map(|(i, _)| i)
}

/// Verifies an animated export: it parses, carries the clip, and no bone leaves
/// a box far larger than the character at any sample across the clip.
fn assert_animates_cleanly(bytes: &[u8], clip_name: &str, label: &str) {
    // A human is ~1.8 m; 5 m from the origin in any axis is well past anywhere a
    // bone should be, so a limb that has flown off trips this.
    assert_animates_cleanly_within(bytes, clip_name, label, 5.0);
}

fn assert_animates_cleanly_within(bytes: &[u8], clip_name: &str, label: &str, limit: f32) {
    let document = m2m_io::glb::read(bytes).expect("the export reads");
    let clip = document
        .clips
        .iter()
        .find(|c| c.name == clip_name)
        .unwrap_or_else(|| panic!("{label}: no clip {clip_name}"));
    assert!(
        clip.channels
            .iter()
            .any(|c| c.path == m2m_io::glb::Path::Rotation),
        "{label}: the clip drives no rotations"
    );

    let samples = 16;
    for step in 0..=samples {
        let t = clip.duration * step as f32 / samples as f32;
        for (bone, position) in bone_positions_at(&document, clip, t).iter().enumerate() {
            assert!(
                position.is_finite() && position.abs().max_element() < limit,
                "{label}: bone {bone} reached {position:?} at t={t:.2}s"
            );
        }
    }
}

const HUMAN_LIB: &str = "legacy/static/animations/human-base-animations.glb";

#[test]
#[ignore = "runs the full fit+bind+retarget+export pipeline; slow"]
fn a_clip_retargets_cleanly_onto_a_t_pose_human() {
    let glb = animated_export(
        "human",
        "legacy/static/models/model-human.glb",
        HUMAN_LIB,
        "Chop_Tree",
    );
    assert_animates_cleanly(&glb, "Chop_Tree", "t-pose");
}

#[test]
#[ignore = "runs the full fit+bind+retarget+export pipeline; slow"]
fn a_t_pose_clip_retargets_cleanly_onto_an_a_pose_human() {
    // The case the whole epic is for: a T-pose Mixamo clip on an A-pose bind.
    let glb = animated_export(
        "human",
        "legacy/static/test-files/bone-correction-tests/human-a-pose.glb",
        HUMAN_LIB,
        "Chop_Tree",
    );
    assert_animates_cleanly(&glb, "Chop_Tree", "a-pose");
}

#[test]
#[ignore = "runs the full fit+bind+retarget+export pipeline; slow"]
fn a_clip_retargets_cleanly_onto_a_non_human_rig() {
    // P3-P6: the dragon reorients its wings and legs by ~44°, so its animation
    // exercises the pose-matched rotations on non-human limbs. A larger box —
    // a dragon is a big creature — but the same "nothing flies off" bar.
    let lib = "legacy/static/animations/dragon-animations.glb";
    let clip = first_clip(lib);
    let glb = animated_export(
        "dragon",
        "legacy/static/models/model-dragon.glb",
        lib,
        &clip,
    );
    assert_animates_cleanly_within(&glb, &clip, "dragon", 12.0);
}

#[test]
#[ignore = "runs the full fit+bind+retarget+export pipeline; slow"]
fn a_clip_retargets_cleanly_onto_the_rhino() {
    // P3-13: the rhino is a new authored quadruped template with its own
    // animation library (its native clip, renamed to the template's bones).
    let lib = "legacy/static/animations/rhino-animations.glb";
    let clip = first_clip(lib);
    let glb = animated_export("rhino", "legacy/static/models/model-rhino.glb", lib, &clip);
    assert_animates_cleanly_within(&glb, &clip, "rhino", 12.0);
}

#[test]
#[ignore = "runs the full fit+bind+retarget+export pipeline; slow"]
fn a_clip_retargets_cleanly_onto_the_buffalo() {
    let lib = "legacy/static/animations/buffalo-animations.glb";
    let clip = first_clip(lib);
    let glb = animated_export(
        "buffalo",
        "legacy/static/models/model-buffalo.glb",
        lib,
        &clip,
    );
    assert_animates_cleanly_within(&glb, &clip, "buffalo", 12.0);
}
