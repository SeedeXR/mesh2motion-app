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
