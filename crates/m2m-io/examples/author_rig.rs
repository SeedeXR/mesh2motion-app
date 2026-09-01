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

fn mapping(creature: &str) -> Map {
    match creature {
        "rhino" => RHINO,
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
    let ground = map
        .iter()
        .zip(&raw)
        .filter(|((_, src, _, _), _)| !synthetic(src))
        .map(|(_, p)| p.y)
        .fold(f32::INFINITY, f32::min);
    for (i, (_, src, parent, _)) in map.iter().enumerate() {
        if *src == "@ground" {
            let p = raw[index_of(parent).expect("ground bone has a parent")];
            raw[i] = glam::Vec3::new(p.x, ground, p.z);
        } else if let Some(from) = src.strip_prefix("@mirror:") {
            let p = raw[index_of(from).expect("mirrored-from bone precedes it")];
            raw[i] = glam::Vec3::new(-p.x, p.y, p.z);
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
    let clips = doc.clips.len();
    doc.primitives.clear(); // library is skeleton + clips only, no mesh
    let bytes = glb::write(&doc).unwrap();
    std::fs::write(output, &bytes).unwrap();
    println!(
        "{creature} anim: {renamed} bones renamed, {clips} clip(s), wrote {} bytes to {output}",
        bytes.len()
    );
}
