//! Authors a clean skeleton-only fit-template glb from a rigged character glb.
//!
//! The shipped fit templates (`assets/rigs/rig-*.glb`) are skeleton-only: a skin
//! whose joints are cleanly named bones in typed chains, no mesh. A downloaded
//! character rig, by contrast, carries helper bones (IK targets, pole targets,
//! duplicate `.001` joints) and its own naming. This tool selects the
//! deformation bones from a character, renames them to a template's convention,
//! reparents them into clean chains, grounds the feet at y=0, and writes a
//! skeleton-only glb with identity rest rotations — the same shape the fitter's
//! `rest_pose` reads.
//!
//!     cargo run -p m2m-io --release --example author_rig -- <character.glb> <creature> <out.glb>
//!
//! `creature` selects one of the MAPPINGS below. Each row is
//! `(template_bone, source_bone, template_parent)`, parents-before-children, with
//! `""` parent meaning the root hangs off the Armature node.

use m2m_io::glb::{self, Document, GlbReport, Node, Skin, Trs};

/// One creature's bone map, per template bone:
/// `(template name, skeleton source, template parent, animation source)`.
///
/// `skeleton source` builds the rest pose — it may be `@mirror:<bone>` (reflect
/// another template bone across x, so the rest pose is symmetric even when the
/// source rig is not) or `@ground` (drop to the floor below the parent).
/// `animation source` is the source bone whose motion this template bone takes;
/// it is the *real* per-side bone (never a mirror), and `""` when the bone is
/// synthetic and carries no animation.
type Map = &'static [(&'static str, &'static str, &'static str, &'static str)];

/// Rhino: a large graviportal quadruped. Drops the source rig's `.001`
/// duplicates and its redundant per-limb pelvis bones; reparents the back legs
/// straight onto the root. The rest pose mirrors the left side onto the right
/// (the source rig is asymmetric), but each side animates from its own bone.
const RHINO: Map = &[
    ("root", "root", "", "root"),
    ("spine_1", "spine_01", "root", "spine_01"),
    ("spine_2", "spine_02", "spine_1", "spine_02"),
    ("spine_3", "chest", "spine_2", "chest"),
    ("neck_1", "neck_01", "spine_3", "neck_01"),
    ("head", "head", "neck_1", "head"),
    ("jaw", "jaw", "head", "jaw"),
    ("ear_l", "l.ear", "head", "l.ear"),
    ("ear_r", "@mirror:ear_l", "head", "r.ear"),
    ("front_shoulder_l", "l.clav", "spine_3", "l.clav"),
    ("front_upper_l", "l.shldr", "front_shoulder_l", "l.shldr"),
    ("front_lower_l", "l.elbow", "front_upper_l", "l.elbow"),
    ("front_foot_l", "l.wrist", "front_lower_l", "l.wrist"),
    (
        "front_shoulder_r",
        "@mirror:front_shoulder_l",
        "spine_3",
        "r.clav",
    ),
    (
        "front_upper_r",
        "@mirror:front_upper_l",
        "front_shoulder_r",
        "r.shldr",
    ),
    (
        "front_lower_r",
        "@mirror:front_lower_l",
        "front_upper_r",
        "r.elbow",
    ),
    (
        "front_foot_r",
        "@mirror:front_foot_l",
        "front_lower_r",
        "r.wrist",
    ),
    ("back_upper_l", "l.hip", "root", "l.hip"),
    ("back_lower_l", "l.knee", "back_upper_l", "l.knee"),
    ("back_foot_l", "l_.ankle", "back_lower_l", "l_.ankle"),
    // The source rig's back legs stop at the hock; extend to the ground so all
    // four feet reach the floor, as the horse template's do. Synthetic: no anim.
    ("back_toe_l", "@ground", "back_foot_l", ""),
    ("back_upper_r", "@mirror:back_upper_l", "root", "r.hip"),
    (
        "back_lower_r",
        "@mirror:back_lower_l",
        "back_upper_r",
        "r.knee",
    ),
    (
        "back_foot_r",
        "@mirror:back_foot_l",
        "back_lower_r",
        "r_.ankle",
    ),
    ("back_toe_r", "@mirror:back_toe_l", "back_foot_r", ""),
    ("tail_1", "tail_01", "root", "tail_01"),
    ("tail_2", "tail_02", "tail_1", "tail_02"),
    ("tail_3", "tail_03", "tail_2", "tail_03"),
    ("tail_4", "tail_04", "tail_3", "tail_04"),
    ("tail_5", "tail_05", "tail_4", "tail_05"),
];

/// Buffalo: a large bovine quadruped. Its source rig is bilaterally symmetric,
/// so each side maps from its own bone (no `@mirror`); the IK/pole-target helper
/// bones are dropped and every leg is extended to the floor.
#[rustfmt::skip]
const BUFFALO: Map = &[
    // `Back` (the croup hub the source hangs legs and tail off) is the root; the
    // source's low `Body` anchor is dropped. The spine is the three Torso bones,
    // which sit inside the mesh — `Back` itself is a high rear point that the
    // uniform spine fit leaves outside, so it must not be a spine joint.
    ("root", "Back", "", "Back"),
    ("spine_1", "Torso", "root", "Torso"),
    ("spine_2", "Torso2", "spine_1", "Torso2"),
    ("spine_3", "Torso3", "spine_2", "Torso3"),
    ("neck_1", "Neck1", "spine_3", "Neck1"),
    ("neck_2", "Neck2", "neck_1", "Neck2"),
    ("neck_3", "Neck3", "neck_2", "Neck3"),
    ("head", "Head", "neck_3", "Head"),
    ("front_shoulder_l", "FrontShoulder.L", "spine_2", "FrontShoulder.L"),
    ("front_upper_l", "FrontUpperLeg.L", "front_shoulder_l", "FrontUpperLeg.L"),
    ("front_lower_l", "FrontLowerLeg.L", "front_upper_l", "FrontLowerLeg.L"),
    ("front_foot_l", "@ground", "front_lower_l", ""),
    ("front_shoulder_r", "FrontShoulder.R", "spine_2", "FrontShoulder.R"),
    ("front_upper_r", "FrontUpperLeg.R", "front_shoulder_r", "FrontUpperLeg.R"),
    ("front_lower_r", "FrontLowerLeg.R", "front_upper_r", "FrontLowerLeg.R"),
    ("front_foot_r", "@ground", "front_lower_r", ""),
    ("back_hip_l", "BackShoulder.L", "root", "BackShoulder.L"),
    ("back_upper_l", "BackLeg.L", "back_hip_l", "BackLeg.L"),
    ("back_lower_l", "BackUpperLeg.L", "back_upper_l", "BackUpperLeg.L"),
    ("back_foot_l", "BackLowerLeg.L", "back_lower_l", "BackLowerLeg.L"),
    ("back_toe_l", "@ground", "back_foot_l", ""),
    ("back_hip_r", "BackShoulder.R", "root", "BackShoulder.R"),
    ("back_upper_r", "BackLeg.R", "back_hip_r", "BackLeg.R"),
    ("back_lower_r", "BackUpperLeg.R", "back_upper_r", "BackUpperLeg.R"),
    ("back_foot_r", "BackLowerLeg.R", "back_lower_r", "BackLowerLeg.R"),
    ("back_toe_r", "@ground", "back_foot_r", ""),
    ("tail_1", "Tail1", "root", "Tail1"),
    ("tail_2", "Tail2", "tail_1", "Tail2"),
    ("tail_3", "Tail3", "tail_2", "Tail3"),
    ("tail_4", "Tail4", "tail_3", "Tail4"),
    ("tail_5", "Tail5", "tail_4", "Tail5"),
    ("tail_6", "Tail6", "tail_5", "Tail6"),
    ("tail_7", "Tail7", "tail_6", "Tail7"),
];

/// Hyena: a digitigrade canine. Detailed source rig (122 bones) trimmed to the
/// deformation set — its many `_end` leaf tips, tongue and mouth bones are
/// dropped. Bilaterally symmetric, so each side maps from its own bone. The toe
/// (`Digit11`) is the ground contact, so no ground extension is needed.
#[rustfmt::skip]
const HYENA: Map = &[
    ("root", "Root", "", "Root"),
    ("spine_1", "Pelvis", "root", "Pelvis"),
    ("spine_2", "Spine1", "spine_1", "Spine1"),
    ("spine_3", "Spine2", "spine_2", "Spine2"),
    ("spine_4", "Chest", "spine_3", "Chest"),
    ("neck_1", "Neck1", "spine_4", "Neck1"),
    ("neck_2", "Neck2", "neck_1", "Neck2"),
    ("neck_3", "Neck3", "neck_2", "Neck3"),
    ("head", "Head", "neck_3", "Head"),
    ("jaw", "Jaw", "head", "Jaw"),
    ("ear_l1", "EarL1", "head", "EarL1"),
    ("ear_l2", "EarL2", "ear_l1", "EarL2"),
    ("ear_l3", "EarL3", "ear_l2", "EarL3"),
    ("ear_r1", "EarR1", "head", "EarR1"),
    ("ear_r2", "EarR2", "ear_r1", "EarR2"),
    ("ear_r3", "EarR3", "ear_r2", "EarR3"),
    ("front_collar_l", "LegFLCollarbone", "spine_4", "LegFLCollarbone"),
    ("front_upper_l", "LegFL1", "front_collar_l", "LegFL1"),
    ("front_lower_l", "LegFL2", "front_upper_l", "LegFL2"),
    ("front_knee_l", "LegFL3", "front_lower_l", "LegFL3"),
    ("front_ankle_l", "LegFLAnkle", "front_knee_l", "LegFLAnkle"),
    ("front_foot_l", "LegFLDigit11", "front_ankle_l", "LegFLDigit11"),
    ("front_collar_r", "LegFRCollarbone", "spine_4", "LegFRCollarbone"),
    ("front_upper_r", "LegFR1", "front_collar_r", "LegFR1"),
    ("front_lower_r", "LegFR2", "front_upper_r", "LegFR2"),
    ("front_knee_r", "LegFR3", "front_lower_r", "LegFR3"),
    ("front_ankle_r", "LegFRAnkle", "front_knee_r", "LegFRAnkle"),
    ("front_foot_r", "LegFRDigit11", "front_ankle_r", "LegFRDigit11"),
    ("back_upper_l", "LegBL1", "spine_1", "LegBL1"),
    ("back_lower_l", "LegBL2", "back_upper_l", "LegBL2"),
    ("back_knee_l", "LegBL3", "back_lower_l", "LegBL3"),
    ("back_ankle_l", "LegBLAnkle", "back_knee_l", "LegBLAnkle"),
    ("back_foot_l", "LegBLDigit11", "back_ankle_l", "LegBLDigit11"),
    // The back legs are a segment shorter than the front, so their toe lands
    // above the floor after fitting; a ground-contact bone reaches it down.
    ("back_toe_l", "@ground", "back_foot_l", ""),
    ("back_upper_r", "LegBR1", "spine_1", "LegBR1"),
    ("back_lower_r", "LegBR2", "back_upper_r", "LegBR2"),
    ("back_knee_r", "LegBR3", "back_lower_r", "LegBR3"),
    ("back_ankle_r", "LegBRAnkle", "back_knee_r", "LegBRAnkle"),
    ("back_foot_r", "LegBRDigit11", "back_ankle_r", "LegBRDigit11"),
    ("back_toe_r", "@ground", "back_foot_r", ""),
    ("tail_1", "Tail1", "spine_1", "Tail1"),
    ("tail_2", "Tail2", "tail_1", "Tail2"),
    ("tail_3", "Tail3", "tail_2", "Tail3"),
    ("tail_4", "Tail4", "tail_3", "Tail4"),
];

/// Elephant: a large graviportal quadruped with a trunk. Trimmed from a 153-bone
/// Maya rig thick with helper joints (`jointNN`, eyes, control nulls). Its back
/// legs hang off disconnected roots in the source, so they are reparented onto
/// the root here; the world positions are preserved either way. Symmetric.
#[rustfmt::skip]
const ELEPHANT: Map = &[
    ("root", "rootJ", "", "rootJ"),
    ("spine_1", "spineJ_1", "root", "spineJ_1"),
    ("spine_2", "spineJ_2", "spine_1", "spineJ_2"),
    ("spine_3", "spineJ_3", "spine_2", "spineJ_3"),
    ("spine_4", "spineJ_4", "spine_3", "spineJ_4"),
    ("spine_5", "spineJ_5", "spine_4", "spineJ_5"),
    ("spine_6", "spineJ_6", "spine_5", "spineJ_6"),
    // Marker at the domed back over the shoulders; keeps the fit from over-scaling.
    ("back_ridge", "@raise:spine_5:0.20", "spine_5", ""),
    ("head", "headJ", "spine_6", "headJ"),
    ("jaw_1", "mouthJ1", "head", "mouthJ1"),
    ("jaw_2", "mouthJ2", "jaw_1", "mouthJ2"),
    ("jaw_3", "mouthJ3", "jaw_2", "mouthJ3"),
    ("jaw_4", "mouthJ4", "jaw_3", "mouthJ4"),
    ("jaw_5", "mouthJ5", "jaw_4", "mouthJ5"),
    ("trunk_1", "trunkJ_", "head", "trunkJ_"),
    ("trunk_2", "trunkJ_1", "trunk_1", "trunkJ_1"),
    ("trunk_3", "trunkJ_2", "trunk_2", "trunkJ_2"),
    ("trunk_4", "trunkJ_3", "trunk_3", "trunkJ_3"),
    ("trunk_5", "trunkJ_4", "trunk_4", "trunkJ_4"),
    ("trunk_6", "trunkJ_5", "trunk_5", "trunkJ_5"),
    ("trunk_7", "trunkJ_6", "trunk_6", "trunkJ_6"),
    ("trunk_8", "trunkJ_7", "trunk_7", "trunkJ_7"),
    ("trunk_9", "trunkJ_8", "trunk_8", "trunkJ_8"),
    ("trunk_10", "trunkJ_9", "trunk_9", "trunkJ_9"),
    ("trunk_11", "trunkJ_10", "trunk_10", "trunkJ_10"),
    ("trunk_12", "trunkJ_11", "trunk_11", "trunkJ_11"),
    ("trunk_13", "trunkJ_12", "trunk_12", "trunkJ_12"),
    ("tusk_l1", "leftTusk1", "head", "leftTusk1"),
    ("tusk_l2", "leftTusk2", "tusk_l1", "leftTusk2"),
    ("tusk_r1", "rightTusk1", "head", "rightTusk1"),
    ("tusk_r2", "rightTusk2", "tusk_r1", "rightTusk2"),
    ("ear_l", "leftEarJ1", "head", "leftEarJ1"),
    ("ear_r", "rightEarJ1", "head", "rightEarJ1"),
    ("front_hip_l", "leftFrontLegJ_1", "spine_5", "leftFrontLegJ_1"),
    ("front_upper_l", "leftFrontThigh", "front_hip_l", "leftFrontThigh"),
    ("front_lower_l", "leftFrontLegJ_3", "front_upper_l", "leftFrontLegJ_3"),
    ("front_ankle_l", "leftFrontLegJ_4", "front_lower_l", "leftFrontLegJ_4"),
    ("front_foot_l", "leftFrontLegJ_5", "front_ankle_l", "leftFrontLegJ_5"),
    ("front_hip_r", "rightFrontLegJ_1", "spine_5", "rightFrontLegJ_1"),
    ("front_upper_r", "rightFrontThigh", "front_hip_r", "rightFrontThigh"),
    ("front_lower_r", "rightFrontLegJ_3", "front_upper_r", "rightFrontLegJ_3"),
    ("front_ankle_r", "rightFrontLegJ_4", "front_lower_r", "rightFrontLegJ_4"),
    ("front_foot_r", "rightFrontLegJ_5", "front_ankle_r", "rightFrontLegJ_5"),
    ("back_hip_l", "lefBackLegJ_1", "root", "lefBackLegJ_1"),
    ("back_upper_l", "leftRearThigh", "back_hip_l", "leftRearThigh"),
    ("back_lower_l", "lefBackLegJ_3", "back_upper_l", "lefBackLegJ_3"),
    ("back_ankle_l", "lefBackLegJ_4", "back_lower_l", "lefBackLegJ_4"),
    ("back_foot_l", "lefBackLegJ_5", "back_ankle_l", "lefBackLegJ_5"),
    ("back_hip_r", "lefBackLegJ_11", "root", "lefBackLegJ_11"),
    ("back_upper_r", "rightRearThigh", "back_hip_r", "rightRearThigh"),
    ("back_lower_r", "lefBackLegJ_3.001", "back_upper_r", "lefBackLegJ_3.001"),
    ("back_ankle_r", "lefBackLegJ_4.001", "back_lower_r", "lefBackLegJ_4.001"),
    ("back_foot_r", "lefBackLegJ_5.001", "back_ankle_r", "lefBackLegJ_5.001"),
    ("tail_1", "tailJ_", "root", "tailJ_"),
    ("tail_2", "tailJ_1", "tail_1", "tailJ_1"),
    ("tail_3", "tailJ_2", "tail_2", "tailJ_2"),
    ("tail_4", "tailJ_3", "tail_3", "tailJ_3"),
    ("tail_5", "tailJ_4", "tail_4", "tailJ_4"),
];

/// Giraffe: the source is an auto-rig with meaningless `Bone.0NN` names and a
/// flat hierarchy (every bone hangs straight off the armature), so the map both
/// renames the deformation bones and rebuilds the chains. The long neck is five
/// bones; the right side mirrors the left for a symmetric rest (the source legs
/// sit at slightly different x). The source carries no animation of its own, so
/// the giraffe's library is retargeted from a quadruped, not authored here.
const GIRAFFE: Map = &[
    ("root", "Bone.029", "", "Bone.029"),
    ("spine_1", "Bone", "root", "Bone"),
    ("spine_2", "Bone.009", "spine_1", "Bone.009"),
    ("neck_1", "Bone.030", "spine_2", "Bone.030"),
    ("neck_2", "Bone.031", "neck_1", "Bone.031"),
    ("neck_3", "Bone.032", "neck_2", "Bone.032"),
    ("neck_4", "Bone.033", "neck_3", "Bone.033"),
    ("neck_5", "Bone.034", "neck_4", "Bone.034"),
    ("head", "Bone.035", "neck_5", "Bone.035"),
    ("front_upper_l", "Bone.020", "spine_2", "Bone.020"),
    ("front_lower_l", "Bone.021", "front_upper_l", "Bone.021"),
    ("front_foot_l", "Bone.022", "front_lower_l", "Bone.022"),
    (
        "front_upper_r",
        "@mirror:front_upper_l",
        "spine_2",
        "Bone.017",
    ),
    (
        "front_lower_r",
        "@mirror:front_lower_l",
        "front_upper_r",
        "Bone.018",
    ),
    (
        "front_foot_r",
        "@mirror:front_foot_l",
        "front_lower_r",
        "Bone.019",
    ),
    ("back_upper_l", "Bone.026", "root", "Bone.026"),
    ("back_lower_l", "Bone.027", "back_upper_l", "Bone.027"),
    ("back_foot_l", "Bone.028", "back_lower_l", "Bone.028"),
    ("back_upper_r", "@mirror:back_upper_l", "root", "Bone.023"),
    (
        "back_lower_r",
        "@mirror:back_lower_l",
        "back_upper_r",
        "Bone.024",
    ),
    (
        "back_foot_r",
        "@mirror:back_foot_l",
        "back_lower_r",
        "Bone.025",
    ),
    ("tail_1", "Bone.010", "root", "Bone.010"),
    ("tail_2", "Bone.011", "tail_1", "Bone.011"),
    ("tail_3", "Bone.012", "tail_2", "Bone.012"),
    ("tail_4", "Bone.013", "tail_3", "Bone.013"),
    ("tail_5", "Bone.014", "tail_4", "Bone.014"),
];

/// Crow: a bird. The source rig is cleanly named and bilaterally symmetric, so
/// each side maps from its own bone. Front limbs are wings (clavicle → hand),
/// hind limbs are digitigrade bird legs (thigh → toe through the "HorseLink"
/// hock), and the fan of tail feathers is reduced to a single tail bone. The
/// many `WingLeft*`/feather helper bones are dropped to the deformation set.
#[rustfmt::skip]
const CROW: Map = &[
    ("root", "CROW_", "", "CROW_"),
    ("spine_1", "CROW_ Pelvis", "root", "CROW_ Pelvis"),
    ("spine_2", "CROW_ Spine", "spine_1", "CROW_ Spine"),
    ("neck_1", "CROW_ Neck", "spine_2", "CROW_ Neck"),
    ("head", "CROW_ Head", "neck_1", "CROW_ Head"),
    ("leg_upper_l", "CROW_ L Thigh", "spine_1", "CROW_ L Thigh"),
    ("leg_lower_l", "CROW_ L Calf", "leg_upper_l", "CROW_ L Calf"),
    ("leg_ankle_l", "CROW_ L HorseLink", "leg_lower_l", "CROW_ L HorseLink"),
    ("leg_foot_l", "CROW_ L Foot", "leg_ankle_l", "CROW_ L Foot"),
    ("leg_toe_l", "CROW_ L Toe0", "leg_foot_l", "CROW_ L Toe0"),
    ("leg_upper_r", "CROW_ R Thigh", "spine_1", "CROW_ R Thigh"),
    ("leg_lower_r", "CROW_ R Calf", "leg_upper_r", "CROW_ R Calf"),
    ("leg_ankle_r", "CROW_ R HorseLink", "leg_lower_r", "CROW_ R HorseLink"),
    ("leg_foot_r", "CROW_ R Foot", "leg_ankle_r", "CROW_ R Foot"),
    ("leg_toe_r", "CROW_ R Toe0", "leg_foot_r", "CROW_ R Toe0"),
    ("wing_1_l", "CROW_ L Clavicle", "spine_2", "CROW_ L Clavicle"),
    ("wing_2_l", "CROW_ L UpperArm", "wing_1_l", "CROW_ L UpperArm"),
    ("wing_3_l", "CROW_ L Forearm", "wing_2_l", "CROW_ L Forearm"),
    ("wing_4_l", "CROW_ L Hand", "wing_3_l", "CROW_ L Hand"),
    ("wing_1_r", "CROW_ R Clavicle", "spine_2", "CROW_ R Clavicle"),
    ("wing_2_r", "CROW_ R UpperArm", "wing_1_r", "CROW_ R UpperArm"),
    ("wing_3_r", "CROW_ R Forearm", "wing_2_r", "CROW_ R Forearm"),
    ("wing_4_r", "CROW_ R Hand", "wing_3_r", "CROW_ R Hand"),
    ("tail_1", "CrowTailLeftFeatherA", "spine_1", "CrowTailLeftFeatherA"),
];

fn mapping(creature: &str) -> Map {
    match creature {
        "rhino" => RHINO,
        "buffalo" => BUFFALO,
        "hyena" => HYENA,
        "elephant" => ELEPHANT,
        "giraffe" => GIRAFFE,
        "crow" => CROW,
        other => panic!("no mapping for {other:?}"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(creature), Some(output)) = (args.next(), args.next(), args.next())
    else {
        eprintln!("usage: author_rig <character.glb> <creature> <out.glb> [skeleton|anim]");
        std::process::exit(2);
    };
    let mode = args.next().unwrap_or_else(|| "skeleton".into());
    let map = mapping(&creature);

    if mode == "anim" {
        author_anim(&input, map, &creature, &output);
        return;
    }
    if mode == "model" {
        // Scale a plain (skin already baked) mesh into a fixture at the skeleton's
        // metre scale. Kept in the same frame as the skeleton by reading the glb
        // directly rather than round-tripping through Blender again.
        let scale: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let mut doc = glb::read(&std::fs::read(&input).unwrap()).unwrap();
        for prim in &mut doc.primitives {
            for c in &mut prim.positions {
                *c *= scale;
            }
        }
        doc.skins.clear();
        doc.clips.clear();
        for node in &mut doc.nodes {
            node.skin = None;
        }
        std::fs::write(&output, glb::write(&doc).unwrap()).unwrap();
        println!("{creature} model: scaled x{scale}");
        return;
    }

    let doc = glb::read(&std::fs::read(&input).unwrap()).unwrap();
    let world = doc.world_transforms();
    let pos_of = |src: &str| -> glam::Vec3 {
        let idx = doc
            .nodes
            .iter()
            .position(|n| n.name == src)
            .unwrap_or_else(|| panic!("source bone {src:?} not found"));
        world[idx].to_scale_rotation_translation().2
    };

    let index_of = |name: &str| map.iter().position(|(n, _, _, _)| *n == name);

    // World position of every template bone. Real bones map straight to a source
    // joint; the two synthetic sources are resolved in a second pass, in map
    // order, so their dependency (parent for `@ground`, the mirrored-from bone
    // for `@mirror`) is already known:
    //   `@ground`         — dropped straight below its parent to the floor.
    //   `@mirror:<bone>`   — the named bone reflected across x, for a symmetric
    //                        right side even when the source rig is asymmetric.
    let synthetic = |src: &str| src.starts_with('@');
    let mut raw: Vec<glam::Vec3> = map
        .iter()
        .map(|(_, src, _, _)| {
            if synthetic(src) {
                glam::Vec3::ZERO
            } else {
                pos_of(src)
            }
        })
        .collect();
    // Ground on the lowest real, non-root bone: the feet, not the root. Some
    // source rigs put the root at the origin, below the body, which would
    // otherwise leave the feet floating above y=0.
    let ground = map
        .iter()
        .zip(&raw)
        .filter(|((_, src, parent, _), _)| !synthetic(src) && !parent.is_empty())
        .map(|(_, p)| p.y)
        .fold(f32::INFINITY, f32::min);
    for (i, (_, src, parent, _)) in map.iter().enumerate() {
        if *src == "@ground" {
            let p = raw[index_of(parent).expect("ground bone has a parent")];
            raw[i] = glam::Vec3::new(p.x, ground, p.z);
        } else if let Some(from) = src.strip_prefix("@mirror:") {
            let p = raw[index_of(from).expect("mirrored-from bone precedes it")];
            raw[i] = glam::Vec3::new(-p.x, p.y, p.z);
        } else if let Some(spec) = src.strip_prefix("@raise:") {
            // `@raise:<ref>:<dy>` — a marker lifted `dy` above a reference bone,
            // so a template whose bones sit below the mesh's dome (the elephant's
            // back rides above its mid-body spine) spans the mesh height and the
            // uniform fit does not over-scale.
            let (from, dy) = spec.split_once(':').expect("@raise:<ref>:<dy>");
            let p = raw[index_of(from).expect("raised-from bone precedes it")];
            raw[i] = glam::Vec3::new(p.x, p.y + dy.parse::<f32>().expect("a number"), p.z);
        }
    }
    let world_pos: Vec<glam::Vec3> = raw
        .iter()
        .map(|p| *p - glam::Vec3::new(0.0, ground, 0.0))
        .collect();

    // Node 0 is the Armature the root bone hangs off, as the shipped rigs have.
    let mut nodes = vec![Node {
        name: "Armature".into(),
        parent: None,
        transform: identity(),
        skin: None,
    }];
    for (i, (name, _, parent, _)) in map.iter().enumerate() {
        let parent_world = if parent.is_empty() {
            glam::Vec3::ZERO
        } else {
            world_pos[index_of(parent).expect("parent precedes child")]
        };
        // Identity rest rotations, so a local offset is just world minus parent.
        let local = world_pos[i] - parent_world;
        let parent_node = if parent.is_empty() {
            0
        } else {
            index_of(parent).unwrap() + 1
        };
        nodes.push(Node {
            name: (*name).into(),
            parent: Some(parent_node),
            transform: Trs {
                translation: local.to_array(),
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
            skin: None,
        });
    }

    let joints: Vec<usize> = (1..=map.len()).collect();
    let document = Document {
        nodes,
        primitives: vec![],
        materials: vec![],
        skins: vec![Skin {
            joints,
            inverse_bind_matrices: vec![],
        }],
        clips: vec![],
        report: GlbReport::default(),
    };

    let bytes = glb::write(&document).unwrap();
    std::fs::write(&output, &bytes).unwrap();
    println!(
        "{creature}: {} bones, grounded by {ground:.3}, wrote {} bytes to {output}",
        map.len(),
        bytes.len()
    );
}

fn identity() -> Trs {
    Trs {
        translation: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

/// Builds a creature's animation library from the character's own clips.
///
/// The character carries its native animation on the source rig's bone names.
/// The retargeter maps clips onto a fitted template by name, so this renames the
/// mapped source bones to the template's convention (channels are node-index
/// based, so the rename does not disturb them) and drops the mesh — leaving a
/// skeleton + clips glb the app loads as `<creature>-animations.glb`.
fn author_anim(input: &str, map: Map, creature: &str, output: &str) {
    let mut doc = glb::read(&std::fs::read(input).unwrap()).unwrap();
    let rename: std::collections::HashMap<&str, &str> = map
        .iter()
        .filter(|(_, _, _, anim)| !anim.is_empty())
        .map(|(name, _, _, anim)| (*anim, *name))
        .collect();
    let mut renamed = 0;
    for node in &mut doc.nodes {
        if let Some(&to) = rename.get(node.name.as_str()) {
            node.name = to.to_owned();
            renamed += 1;
        }
    }
    if doc.skins.is_empty() {
        panic!("the character has no skin to carry the clip");
    }
    // Keep rotation channels only. A source clip's translations are the bone's
    // absolute position in the source's units (often FBX centimetres), which do
    // not carry across to a fitted skeleton in normalised units — applied raw
    // they fling bones off. Canned preview clips animate in place, so rotations
    // are all that is wanted; the retargeter is a rotation retarget anyway.
    let clips = doc.clips.len();
    for clip in &mut doc.clips {
        clip.channels.retain(|c| c.path == glb::Path::Rotation);
    }
    doc.primitives.clear(); // library is skeleton + clips only, no mesh
    let bytes = glb::write(&doc).unwrap();
    std::fs::write(output, &bytes).unwrap();
    println!(
        "{creature} anim: {renamed} bones renamed, {clips} clip(s), wrote {} bytes to {output}",
        bytes.len()
    );
}
