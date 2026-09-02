//! Moving an animation from one skeleton onto another.
//!
//! The acceptance property is the one a verbatim track copy breaks. Our human
//! rig and a Mixamo rig hold five bones — `thigh_l`, `thigh_r`, `calf_l`,
//! `calf_r`, `foot_r` — about **180° apart at rest**, so copying a rotation
//! track across would put the legs on backwards. What must hold is that the
//! retargeted skeleton *points the same way in the world* as the source did.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use m2m_rig::automap::Skeleton;
use m2m_rig::retarget::{retarget, Clip, RestRotations, RotationTrack};

/// A two-bone skeleton: a root and a limb hanging off it.
fn two_bone(limb_rest: Quat) -> (Skeleton, RestRotations) {
    (
        Skeleton {
            names: vec!["root".into(), "limb".into()],
            parents: vec![None, Some(0)],
            positions: vec![Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)],
        },
        RestRotations {
            local: vec![Quat::IDENTITY, limb_rest],
        },
    )
}

/// World rotation of a bone under a clip, at a given time.
fn world_at(
    skeleton: &Skeleton,
    rest: &RestRotations,
    clip: &Clip,
    bone: usize,
    time: f32,
) -> Quat {
    let mut world = vec![Quat::IDENTITY; skeleton.names.len()];
    for index in 0..skeleton.names.len() {
        let local = clip
            .tracks
            .iter()
            .find(|t| t.bone == index)
            .and_then(|track| {
                track
                    .times
                    .iter()
                    .position(|t| (t - time).abs() < 1e-6)
                    .and_then(|i| track.rotations.get(i).copied())
            })
            .unwrap_or_else(|| rest.local[index]);
        world[index] = match skeleton.parents[index] {
            Some(parent) => world[parent] * local,
            None => local,
        };
    }
    world[bone]
}

/// Two rigs holding a limb 180° apart at rest perform the **same motion**.
///
/// **This is the whole point, and my first version of this test asserted the
/// wrong thing.** It required the two limbs to end up pointing the same way in
/// the world, which is false by construction: at rest each rig's limb points
/// where its own rest pose puts it, and these two rest poses are 180° apart.
/// Retargeting preserves the motion a bone makes *away from its rest pose*, not
/// its absolute orientation — a rig whose thigh bone points down and one whose
/// points up are the same leg described two ways.
///
/// So what must hold is that the change in world orientation between two
/// instants is identical on both sides. That is not tautological: it goes
/// through composing the source's world rotation, transferring the motion, and
/// converting back to the target's local frame, which is where the errors are.
#[test]
fn a_limb_held_backwards_at_rest_performs_the_same_motion() {
    let (source, source_rest) = two_bone(Quat::IDENTITY);
    let (target, target_rest) = two_bone(Quat::from_rotation_z(std::f32::consts::PI));

    // The source swings its limb a quarter turn about X.
    let swing = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let clip = Clip {
        name: "swing".into(),
        tracks: vec![RotationTrack {
            bone: 1,
            times: vec![0.0, 1.0],
            rotations: vec![Quat::IDENTITY, swing],
        }],
    };
    let mapping: HashMap<usize, usize> = [(0, 0), (1, 1)].into_iter().collect();

    let (out, report) = retarget(
        &source,
        &source_rest,
        &target,
        &target_rest,
        &mapping,
        &clip,
    );
    assert_eq!(report.unmapped_tracks, 0);

    // At rest the target sits at its own rest pose, 180° from the source's.
    let target_at_rest = world_at(&target, &target_rest, &out, 1, 0.0);
    assert!(
        target_at_rest.dot(target_rest.local[1]).abs() > 0.999,
        "at t=0 the target should hold its own rest pose"
    );

    // The motion from t=0 to t=1 is the same on both sides.
    let source_motion = world_at(&source, &source_rest, &clip, 1, 1.0)
        * world_at(&source, &source_rest, &clip, 1, 0.0).inverse();
    let target_motion = world_at(&target, &target_rest, &out, 1, 1.0)
        * world_at(&target, &target_rest, &out, 1, 0.0).inverse();
    let agreement = source_motion.dot(target_motion).abs();
    assert!(
        agreement > 0.999,
        "the target moved {:.1}° differently from the source",
        2.0 * agreement.clamp(-1.0, 1.0).acos().to_degrees()
    );

    // And the local track really differs from the source's, so this is not a
    // verbatim copy that happens to work.
    let target_track = out.tracks.iter().find(|t| t.bone == 1).expect("a track");
    assert!(
        target_track.rotations[1].dot(swing).abs() < 0.999,
        "the target's local rotation equals the source's, so nothing was compensated"
    );
}

/// Key times survive exactly.
///
/// The time axis is the thing that goes wrong silently: a clip can keep every
/// count and still play at the wrong speed.
#[test]
fn the_time_axis_survives_retargeting() {
    let (source, source_rest) = two_bone(Quat::IDENTITY);
    let (target, target_rest) = two_bone(Quat::from_rotation_y(0.3));
    let times = vec![0.0, 0.25, 0.5, 1.0, 2.5];
    let clip = Clip {
        name: "walk".into(),
        tracks: vec![RotationTrack {
            bone: 1,
            times: times.clone(),
            rotations: times
                .iter()
                .map(|t| Quat::from_rotation_x(t * 0.5))
                .collect(),
        }],
    };
    let mapping: HashMap<usize, usize> = [(1, 1)].into_iter().collect();

    let (out, _) = retarget(
        &source,
        &source_rest,
        &target,
        &target_rest,
        &mapping,
        &clip,
    );
    assert_eq!(out.name, "walk", "the clip keeps its name");
    for track in &out.tracks {
        assert_eq!(track.times, times, "bone {} lost its key times", track.bone);
        assert_eq!(track.rotations.len(), times.len());
    }
    assert!(
        (out.duration() - 2.5).abs() < 1e-6,
        "duration {}",
        out.duration()
    );
}

/// A bone the mapping does not cover keeps its rest pose and is reported.
#[test]
fn an_unmapped_bone_keeps_its_rest_pose_and_is_counted() {
    let (source, source_rest) = two_bone(Quat::IDENTITY);
    let (target, target_rest) = two_bone(Quat::from_rotation_z(0.7));
    let clip = Clip {
        name: "partial".into(),
        tracks: vec![RotationTrack {
            bone: 1,
            times: vec![0.0],
            rotations: vec![Quat::from_rotation_x(1.0)],
        }],
    };

    let (out, report) = retarget(
        &source,
        &source_rest,
        &target,
        &target_rest,
        &HashMap::new(),
        &clip,
    );
    assert_eq!(report.unmapped_tracks, 1, "the driven bone maps nowhere");

    let track = out.tracks.iter().find(|t| t.bone == 1).expect("a track");
    assert!(
        track.rotations[0].dot(target_rest.local[1]).abs() > 0.999,
        "an unmapped bone should sit at its rest rotation"
    );
}

/// A track whose times and values disagree is counted, not trusted.
#[test]
fn a_malformed_track_is_reported_rather_than_used() {
    let (source, source_rest) = two_bone(Quat::IDENTITY);
    let (target, target_rest) = two_bone(Quat::IDENTITY);
    let clip = Clip {
        name: "broken".into(),
        tracks: vec![RotationTrack {
            bone: 1,
            times: vec![0.0, 1.0, 2.0],
            rotations: vec![Quat::IDENTITY],
        }],
    };
    let mapping: HashMap<usize, usize> = [(1, 1)].into_iter().collect();

    let (_, report) = retarget(
        &source,
        &source_rest,
        &target,
        &target_rest,
        &mapping,
        &clip,
    );
    assert_eq!(report.malformed_tracks, 1);
}

/// A skeleton whose parents form a cycle does not hang.
#[test]
fn a_parent_cycle_does_not_hang_retargeting() {
    let cyclic = Skeleton {
        names: vec!["a".into(), "b".into()],
        parents: vec![Some(1), Some(0)],
        positions: vec![Vec3::ZERO, Vec3::Y],
    };
    let rest = RestRotations {
        local: vec![Quat::IDENTITY, Quat::IDENTITY],
    };
    let (out, _) = retarget(
        &cyclic,
        &rest,
        &cyclic,
        &rest,
        &[(0, 0), (1, 1)].into_iter().collect(),
        &Clip {
            name: "c".into(),
            tracks: vec![RotationTrack {
                bone: 0,
                times: vec![0.0],
                rotations: vec![Quat::IDENTITY],
            }],
        },
    );
    assert_eq!(out.tracks.len(), 2, "both bones still produced a track");
}

/// A three-bone chain whose middle bone is both rotated at rest and animated.
fn three_bone(rests: [Quat; 3]) -> (Skeleton, RestRotations) {
    (
        Skeleton {
            names: vec!["root".into(), "mid".into(), "tip".into()],
            parents: vec![None, Some(0), Some(1)],
            positions: vec![
                Vec3::ZERO,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
            ],
        },
        RestRotations {
            local: rests.to_vec(),
        },
    )
}

/// The source's own rest pose is divided out, not assumed to be identity.
///
/// **Written because a mutation survived.** Replacing
/// `source_world * inverse(source_rest_world)` with `source_world` passed every
/// earlier test, because those fixtures gave the source an identity rest pose,
/// which makes the two expressions the same thing. Here the source rests at a
/// real angle, so taking its animated pose as if it were pure motion is wrong.
#[test]
fn the_sources_own_rest_pose_is_divided_out() {
    let source_rest_limb = Quat::from_rotation_y(0.6);
    let (source, source_rest) = two_bone(source_rest_limb);
    let (target, target_rest) = two_bone(Quat::from_rotation_z(1.1));

    // The source holds its rest pose at t=0 and moves away from it at t=1.
    let moved = source_rest_limb * Quat::from_rotation_x(std::f32::consts::FRAC_PI_3);
    let clip = Clip {
        name: "move".into(),
        tracks: vec![RotationTrack {
            bone: 1,
            times: vec![0.0, 1.0],
            rotations: vec![source_rest_limb, moved],
        }],
    };
    let mapping: HashMap<usize, usize> = [(1, 1)].into_iter().collect();
    let (out, _) = retarget(
        &source,
        &source_rest,
        &target,
        &target_rest,
        &mapping,
        &clip,
    );

    // A source sitting at its own rest must leave the target at *its* rest.
    let at_rest = world_at(&target, &target_rest, &out, 1, 0.0);
    let agreement = at_rest.dot(target_rest.local[1]).abs();
    assert!(
        agreement > 0.999,
        "the target moved {:.1}° at a moment the source had not moved at all",
        2.0 * agreement.clamp(-1.0, 1.0).acos().to_degrees()
    );
}

/// A bone's local rotation is divided by its parent's animated world rotation.
///
/// **Written because a mutation survived.** Returning the world rotation as the
/// local one passed every earlier test, because those fixtures never animated a
/// parent: with an identity parent, world and local are equal. Here the middle
/// bone is animated, so the tip's local rotation must have that divided out or
/// the tip inherits its parent's motion twice.
#[test]
fn a_bones_local_rotation_has_its_parent_divided_out() {
    let (source, source_rest) = three_bone([Quat::IDENTITY, Quat::IDENTITY, Quat::IDENTITY]);
    let (target, target_rest) = three_bone([
        Quat::IDENTITY,
        Quat::from_rotation_z(0.4),
        Quat::from_rotation_x(0.2),
    ]);

    // Only the middle bone moves; the tip holds its rest pose.
    let turn = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let clip = Clip {
        name: "bend".into(),
        tracks: vec![RotationTrack {
            bone: 1,
            times: vec![0.0, 1.0],
            rotations: vec![Quat::IDENTITY, turn],
        }],
    };
    let mapping: HashMap<usize, usize> = [(0, 0), (1, 1), (2, 2)].into_iter().collect();
    let (out, _) = retarget(
        &source,
        &source_rest,
        &target,
        &target_rest,
        &mapping,
        &clip,
    );

    // The tip is not driven, so its *local* rotation must stay at rest even
    // though its world rotation swings with the parent.
    let tip = out
        .tracks
        .iter()
        .find(|t| t.bone == 2)
        .expect("a tip track");
    for (index, rotation) in tip.rotations.iter().enumerate() {
        let agreement = rotation.dot(target_rest.local[2]).abs();
        assert!(
            agreement > 0.999,
            "at key {index} the tip's local rotation drifted {:.1}° from rest, \
             so its parent's motion was not divided out",
            2.0 * agreement.clamp(-1.0, 1.0).acos().to_degrees()
        );
    }
}

/// A bone that never moves lands exactly on the **target's** rest translation.
///
/// This is what preserves the target's proportions. Measured across the 87
/// clips in `human-base-animations.glb`, 5,715 of 5,809 translation channels
/// are constant — copying them across would rebuild the target with the
/// source's bone lengths.
#[test]
fn a_bone_that_does_not_move_keeps_the_targets_own_proportions() {
    use m2m_rig::retarget::{retarget_translations, RestTranslations, TranslationTrack};

    let source_rest = RestTranslations {
        local: vec![Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)],
    };
    // The target's limb is half as long: its proportions must survive.
    let target_rest = RestTranslations {
        local: vec![Vec3::ZERO, Vec3::new(0.0, 0.5, 0.0)],
    };
    let tracks = vec![TranslationTrack {
        bone: 1,
        times: vec![0.0, 1.0],
        // Constant, equal to the source's rest offset -- as exporters write it.
        translations: vec![Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
    }];
    let mapping: HashMap<usize, usize> = [(1, 1)].into_iter().collect();

    let out = retarget_translations(&source_rest, &target_rest, &mapping, &tracks, 1.0);
    let track = out.iter().find(|t| t.bone == 1).expect("a track");
    for (index, at) in track.translations.iter().enumerate() {
        assert!(
            at.distance(target_rest.local[1]) < 1e-6,
            "key {index} landed at {at:?}, not the target's rest {:?}",
            target_rest.local[1]
        );
    }
}

/// Root motion is scaled by the height ratio, so a taller character strides
/// further rather than shuffling in place.
#[test]
fn root_motion_is_scaled_by_the_height_ratio() {
    use m2m_rig::retarget::{retarget_translations, RestTranslations, TranslationTrack};

    let source_rest = RestTranslations {
        local: vec![Vec3::ZERO],
    };
    let target_rest = RestTranslations {
        local: vec![Vec3::ZERO],
    };
    // The source's root walks one unit forward.
    let tracks = vec![TranslationTrack {
        bone: 0,
        times: vec![0.0, 1.0],
        translations: vec![Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0)],
    }];
    let mapping: HashMap<usize, usize> = [(0, 0)].into_iter().collect();

    let out = retarget_translations(&source_rest, &target_rest, &mapping, &tracks, 2.0);
    let track = &out[0];
    assert!(track.translations[0].distance(Vec3::ZERO) < 1e-6);
    assert!(
        track.translations[1].distance(Vec3::new(0.0, 0.0, 2.0)) < 1e-6,
        "a twice-as-tall target should stride twice as far, got {:?}",
        track.translations[1]
    );
}

/// The height ratio is measured from the skeletons themselves.
#[test]
fn the_height_ratio_comes_from_the_skeletons() {
    use m2m_rig::retarget::height_scale;

    let short = Skeleton {
        names: vec!["a".into(), "b".into()],
        parents: vec![None, Some(0)],
        positions: vec![Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)],
    };
    let tall = Skeleton {
        names: vec!["a".into(), "b".into()],
        parents: vec![None, Some(0)],
        positions: vec![Vec3::ZERO, Vec3::new(0.0, 3.0, 0.0)],
    };
    assert!((height_scale(&short, &tall) - 3.0).abs() < 1e-6);
    assert!((height_scale(&tall, &short) - 1.0 / 3.0).abs() < 1e-6);
    // A skeleton with no height does not divide by zero.
    assert!(height_scale(&short, &short).is_finite());
}

// ---------------------------------------------------------------------------
// End to end, through the real readers and real assets.
// ---------------------------------------------------------------------------

/// A rig read from a `.glb`: skeleton, rest pose, and the node each bone is.
struct LoadedRig {
    document: m2m_io::glb::Document,
    nodes: Vec<usize>,
    skeleton: Skeleton,
    rotations: m2m_rig::retarget::RestRotations,
}

fn load_rig(relative: &str) -> LoadedRig {
    use glam::Mat4;

    // Rig `.glb` files moved to `assets/rigs/` (P3-3d); other fixtures stay in legacy.
    let path = match relative.strip_prefix("rigs/") {
        Some(rig) => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs/").to_owned() + rig,
        None => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/").to_owned() + relative,
    };
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    let document = m2m_io::glb::read(&bytes).expect("reads");
    let skin = document.skins.first().expect("a skin");

    let local: Vec<Mat4> = document
        .nodes
        .iter()
        .map(|n| {
            Mat4::from_scale_rotation_translation(
                Vec3::from(n.transform.scale),
                Quat::from_array(n.transform.rotation),
                Vec3::from(n.transform.translation),
            )
        })
        .collect();
    let mut world = vec![Mat4::IDENTITY; document.nodes.len()];
    for (index, slot) in world.iter_mut().enumerate() {
        let mut chain = vec![index];
        let mut cursor = index;
        while let Some(parent) = document.nodes[cursor].parent {
            chain.push(parent);
            cursor = parent;
        }
        let mut matrix = Mat4::IDENTITY;
        for &node in chain.iter().rev() {
            matrix *= local[node];
        }
        *slot = matrix;
    }
    let slots: std::collections::HashMap<usize, usize> = skin
        .joints
        .iter()
        .enumerate()
        .map(|(slot, &node)| (node, slot))
        .collect();

    LoadedRig {
        nodes: skin.joints.clone(),
        skeleton: Skeleton {
            names: skin
                .joints
                .iter()
                .map(|&j| document.nodes[j].name.clone())
                .collect(),
            parents: skin
                .joints
                .iter()
                .map(|&j| {
                    document.nodes[j]
                        .parent
                        .and_then(|p| slots.get(&p).copied())
                })
                .collect(),
            positions: skin
                .joints
                .iter()
                .map(|&j| world[j].transform_point3(Vec3::ZERO))
                .collect(),
        },
        rotations: m2m_rig::retarget::RestRotations {
            local: skin
                .joints
                .iter()
                .map(|&j| Quat::from_array(document.nodes[j].transform.rotation))
                .collect(),
        },
        document,
    }
}

/// A real clip moves from a real rig onto another, keeping its time axis.
///
/// Verified independently outside CI: Blender reports the written file as 65
/// bones and 87 actions, with `Chest_Open` at frames **0.00–33.00** — the same
/// range as the source. assimp read 87 animations and 5,655 channels (the
/// library since gained 7 retargeted Mixamo run clips, 94 in total). Neither
/// runs in CI, so the properties they confirmed are asserted here.
#[test]
fn a_real_clip_retargets_between_real_rigs() {
    let source = load_rig("animations/human-base-animations.glb");
    let target = load_rig("test-files/retarget testing/mixamo-sample-rig.glb");

    let known: Vec<m2m_rig::automap::KnownRig> = ["mixamo.json", "rigify.json"]
        .iter()
        .map(|file| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/known-rigs/").to_owned() + file;
            serde_json::from_str(&std::fs::read_to_string(&path).expect("reads")).expect("parses")
        })
        .collect();
    let (target_to_source, _) =
        m2m_rig::automap::map_bones_best(&target.skeleton, &source.skeleton, &known, 0.5);
    let source_to_target: HashMap<usize, usize> =
        target_to_source.iter().map(|(&t, &s)| (s, t)).collect();
    assert_eq!(
        source_to_target.len(),
        65,
        "every target bone should be driven"
    );

    let node_to_bone: HashMap<usize, usize> = source
        .nodes
        .iter()
        .enumerate()
        .map(|(bone, &node)| (node, bone))
        .collect();

    let clip = source
        .document
        .clips
        .iter()
        .find(|c| c.name == "Chest_Open")
        .expect("Chest_Open");
    let tracks: Vec<RotationTrack> = clip
        .channels
        .iter()
        .filter(|c| c.path == m2m_io::glb::Path::Rotation)
        .filter_map(|c| {
            Some(RotationTrack {
                bone: *node_to_bone.get(&c.node)?,
                times: c.times.clone(),
                rotations: c
                    .values
                    .chunks_exact(4)
                    .map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3]))
                    .collect(),
            })
        })
        .collect();
    assert!(!tracks.is_empty(), "the clip has rotation tracks");

    let (moved, report) = retarget(
        &source.skeleton,
        &source.rotations,
        &target.skeleton,
        &target.rotations,
        &source_to_target,
        &Clip {
            name: clip.name.clone(),
            tracks,
        },
    );

    assert_eq!(report.malformed_tracks, 0);
    assert_eq!(
        moved.tracks.len(),
        target.skeleton.names.len(),
        "every target bone gets a track, driven or not"
    );
    // The time axis, which is what goes wrong silently. Blender reads the
    // written file at frames 0.00-33.00, the same as the source: 1.375 s at its
    // 24 fps.
    assert!(
        (moved.duration() - clip.duration).abs() < 1e-4,
        "duration drifted: {} against the source's {}",
        moved.duration(),
        clip.duration
    );
    assert!((clip.duration - 1.375).abs() < 1e-3, "fixture changed");

    // Every rotation is a unit quaternion, or a consumer gets a skewed bone.
    for track in &moved.tracks {
        for rotation in &track.rotations {
            assert!(
                (rotation.length() - 1.0).abs() < 1e-3,
                "bone {} produced a non-unit rotation {rotation:?}",
                track.bone
            );
        }
    }
}
