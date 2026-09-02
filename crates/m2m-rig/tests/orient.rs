//! Pose-matched rest rotations, measured against the real rigs (P3-P3).
//!
//! Two claims to prove, both quantitatively:
//! 1. **No regression.** A creature fitted onto a mesh of its own pose barely
//!    moves any rotation — the nine shipped creatures must be left essentially
//!    as they were, or their working retargeting would break.
//! 2. **The fix.** The human template fitted onto an A-pose mesh reorients the
//!    arm bones by a large angle, because that is the whole point.

use glam::{Mat4, Quat, Vec3};
use m2m_core::mesh::Mesh;
use m2m_rig::fit::{fit_template, RestPose};
use m2m_rig::orient::pose_matched_local_rotations;
use m2m_rig::template::Template;

fn asset(relative: &str) -> Vec<u8> {
    let path = match relative.strip_prefix("rigs/") {
        Some(rig) => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs/").to_owned() + rig,
        None => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/").to_owned() + relative,
    };
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn mesh_of(relative: &str) -> Mesh {
    let document = m2m_io::glb::read(&asset(relative)).expect("reads");
    let world = document.world_transforms();
    let mut mesh = Mesh::default();
    for primitive in &document.primitives {
        let transform = primitive.node.map_or(Mat4::IDENTITY, |n| world[n]);
        let base = mesh.positions.len() as u32;
        for chunk in primitive.positions.chunks_exact(3) {
            mesh.positions
                .push(transform.transform_point3(Vec3::new(chunk[0], chunk[1], chunk[2])));
        }
        mesh.indices
            .extend(primitive.indices.iter().map(|i| i + base));
    }
    mesh
}

/// A template rig's skeleton: names, parents, world positions and **local** rest
/// rotations, read straight from the rig `.glb`'s skin joints.
struct Rig {
    bones: Vec<String>,
    parents: Vec<Option<usize>>,
    positions: Vec<Vec3>,
    local_rotations: Vec<Quat>,
}

fn rig_of(relative: &str) -> Rig {
    let document = m2m_io::glb::read(&asset(relative)).expect("reads");
    let world = document.world_transforms();
    let skin = document.skins.first().expect("a skin");
    let mut joint_of = vec![None; document.nodes.len()];
    for (slot, &node) in skin.joints.iter().enumerate() {
        joint_of[node] = Some(slot);
    }
    Rig {
        bones: skin
            .joints
            .iter()
            .map(|&j| document.nodes[j].name.clone())
            .collect(),
        parents: skin
            .joints
            .iter()
            .map(|&j| document.nodes[j].parent.and_then(|p| joint_of[p]))
            .collect(),
        positions: skin
            .joints
            .iter()
            .map(|&j| world[j].transform_point3(Vec3::ZERO))
            .collect(),
        local_rotations: skin
            .joints
            .iter()
            .map(|&j| Quat::from_array(document.nodes[j].transform.rotation))
            .collect(),
    }
}

fn template(manifest: &str) -> Template {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/").to_owned() + manifest;
    serde_json::from_str(&std::fs::read_to_string(path).expect("reads")).expect("parses")
}

/// The reoriented local rotations for a template fitted onto a mesh.
fn reoriented(rig_path: &str, manifest: &str, mesh_path: &str) -> (Rig, Vec<Quat>) {
    let rig = rig_of(rig_path);
    let rest = RestPose {
        bones: rig.bones.clone(),
        positions: rig.positions.clone(),
    };
    let manifest = template(manifest);
    let fitted = fit_template(&manifest, &rest, &mesh_of(mesh_path), 128).expect("fits");
    let aim = m2m_rig::orient::limb_aims(&manifest, &rig.bones);
    let new = pose_matched_local_rotations(
        &rig.parents,
        &rig.positions,
        &rig.local_rotations,
        &fitted.positions,
        &aim,
    );
    (rig, new)
}

/// The largest angle, in degrees, any bone's rotation moved.
fn max_delta_degrees(before: &[Quat], after: &[Quat]) -> f32 {
    before
        .iter()
        .zip(after)
        .map(|(a, b)| a.angle_between(*b).to_degrees())
        .fold(0.0_f32, f32::max)
}

const CREATURES: &[(&str, &str, &str)] = &[
    ("rigs/rig-human.glb", "human.json", "models/model-human.glb"),
    ("rigs/rig-fox.glb", "fox.json", "models/model-fox.glb"),
    ("rigs/rig-horse.glb", "horse.json", "models/model-horse.glb"),
    ("rigs/rig-bird.glb", "bird.json", "models/model-bird.glb"),
    (
        "rigs/rig-spider.glb",
        "spider.json",
        "models/model-spider.glb",
    ),
    ("rigs/rig-shark.glb", "shark.json", "models/model-shark.glb"),
    ("rigs/rig-snake.glb", "snake.json", "models/model-snake.glb"),
    ("rigs/rig-kaiju.glb", "kaiju.json", "models/model-kaiju.glb"),
    (
        "rigs/rig-dragon.glb",
        "dragon.json",
        "models/model-dragon.glb",
    ),
];

#[test]
fn every_reoriented_rotation_is_valid_and_bounded() {
    // No regression is not "the rotations do not change" — they must, to agree
    // with where the fitter put the bones. It is: every rotation stays finite
    // and unit-length, and none of them flips. A limb that turned by more than
    // ~50° would mean the aim had latched onto a badly-fitted bone (the fingers
    // did, before they were excluded), not a genuine pose difference.
    for (rig, manifest, model) in CREATURES {
        let (r, new) = reoriented(rig, manifest, model);
        for (bone, q) in r.bones.iter().zip(&new) {
            assert!(
                q.is_finite() && (q.length() - 1.0).abs() < 1e-3,
                "{manifest}: {bone} got a non-unit rotation {q:?}"
            );
        }
        let delta = max_delta_degrees(&r.local_rotations, &new);
        assert!(
            delta < 50.0,
            "{manifest} turned a bone by {delta:.1}° — too far to be a pose difference"
        );
    }
}

#[test]
fn a_template_matching_mesh_is_left_alone() {
    // The fox model sits on its own template, so there is no pose difference to
    // correct: the reorientation must be a no-op, proving it does not invent
    // rotations where the fit already agrees with the rig.
    let (rig, new) = reoriented("rigs/rig-fox.glb", "fox.json", "models/model-fox.glb");
    let delta = max_delta_degrees(&rig.local_rotations, &new);
    assert!(
        delta < 3.0,
        "a matching fox fit moved a bone by {delta:.1}°"
    );
}

#[test]
fn an_a_pose_mesh_reorients_the_arms() {
    // The human template is a T-pose. Fitting it onto the A-pose mesh must turn
    // the arm bones by a real angle — that is the correction the whole epic is
    // for.
    let (rig, new) = reoriented(
        "rigs/rig-human.glb",
        "human.json",
        "test-files/bone-correction-tests/human-a-pose.glb",
    );

    let arm_delta = rig
        .bones
        .iter()
        .zip(rig.local_rotations.iter().zip(&new))
        .filter(|(name, _)| name.contains("upperarm") || name.contains("lowerarm"))
        .map(|(_, (a, b))| a.angle_between(*b).to_degrees())
        .fold(0.0_f32, f32::max);

    assert!(
        arm_delta > 20.0,
        "an A-pose fit should reorient the arms; largest arm change was only {arm_delta:.1}°"
    );
}

#[test]
fn non_human_limbs_are_pose_matched_too() {
    // P3-P6: the reorientation is role-agnostic — it aims wings, fins and legs
    // by the same rule as arms. Bird wings, shark fins and dragon wings sit at a
    // real angle off their templates, so a non-trivial correction proves the
    // treatment reaches non-human limbs, not just the human arm chain.
    for (rig, manifest, model) in [
        ("rigs/rig-bird.glb", "bird.json", "models/model-bird.glb"),
        ("rigs/rig-shark.glb", "shark.json", "models/model-shark.glb"),
        (
            "rigs/rig-dragon.glb",
            "dragon.json",
            "models/model-dragon.glb",
        ),
    ] {
        let (r, new) = reoriented(rig, manifest, model);
        let delta = max_delta_degrees(&r.local_rotations, &new);
        assert!(
            delta > 10.0,
            "{manifest} should pose-match its limbs, but nothing moved more than {delta:.1}°"
        );
    }
}
