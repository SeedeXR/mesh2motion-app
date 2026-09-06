//! Moving an animation from one skeleton onto another.
//!
//! # Why the rest pose cannot be ignored
//!
//! A rotation track is expressed in its bone's **local** frame, so copying it
//! from one rig to another only works if the two rigs hold that bone the same
//! way at rest. The legacy's default path does exactly that — it copies the key
//! times and values verbatim and renames the track
//! (`AnimationRetargetService.retarget_animation_clip`) — and reserves a
//! swing/twist path for humans.
//!
//! Measured between our human rig and a Mixamo rig, over the 65 bones their
//! table pairs:
//!
//! | | rest-orientation difference |
//! |---|---|
//! | median bone | **3.8°** |
//! | `thigh_l`, `thigh_r`, `calf_l`, `calf_r`, `foot_r` | **~180°** |
//!
//! A verbatim copy would put the legs on backwards. So this compensates for the
//! rest pose rather than treating it as an optional refinement.
//!
//! # What it does
//!
//! For each key, per mapped bone: take the motion the source made away from its
//! own rest pose, in world space, and apply that same motion to the target's
//! rest pose.
//!
//! ```text
//! motion       = source_animated_world * inverse(source_rest_world)
//! target_world = motion * target_rest_world
//! target_local = inverse(target_parent_animated_world) * target_world
//! ```
//!
//! Working in world space is what makes the 180° legs come out right: the
//! difference between the two rest poses cancels, because each side is measured
//! against its own.

use std::collections::HashMap;

use glam::Quat;

use crate::automap::Skeleton;

/// A skeleton's rest rotations, one local rotation per bone.
///
/// Kept separate from [`Skeleton`], which carries positions, because
/// retargeting rotations needs the orientations and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct RestRotations {
    /// Local rotation of each bone at rest.
    pub local: Vec<Quat>,
}

impl RestRotations {
    /// World rotation of each bone, composed down the hierarchy.
    ///
    /// A cycle in the parents yields identity for the bones caught in it rather
    /// than looping: a skeleton is untrusted input like any other file content.
    pub fn world(&self, skeleton: &Skeleton) -> Vec<Quat> {
        let mut world = vec![Quat::IDENTITY; self.local.len()];
        let mut done = vec![false; self.local.len()];
        for index in 0..self.local.len() {
            let mut chain = Vec::new();
            let mut cursor = Some(index);
            while let Some(current) = cursor {
                if done[current] || chain.contains(&current) {
                    break;
                }
                chain.push(current);
                cursor = skeleton.parents.get(current).copied().flatten();
            }
            for &bone in chain.iter().rev() {
                let parent = skeleton.parents.get(bone).copied().flatten();
                let base = parent
                    .filter(|p| done[*p])
                    .map_or(Quat::IDENTITY, |p| world[p]);
                world[bone] = base * self.local.get(bone).copied().unwrap_or(Quat::IDENTITY);
                done[bone] = true;
            }
        }
        world
    }
}

/// One bone's rotation over time.
#[derive(Debug, Clone, PartialEq)]
pub struct RotationTrack {
    /// Index of the bone this drives.
    pub bone: usize,
    /// Key times, in seconds.
    pub times: Vec<f32>,
    /// One rotation per key.
    pub rotations: Vec<Quat>,
}

/// An animation as a set of rotation tracks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    /// The clip's name.
    pub name: String,
    /// Its tracks.
    pub tracks: Vec<RotationTrack>,
}

impl Clip {
    /// Longest key time in the clip.
    pub fn duration(&self) -> f32 {
        self.tracks
            .iter()
            .filter_map(|t| t.times.last().copied())
            .fold(0.0, f32::max)
    }

    /// Every key time in the clip, sorted and deduplicated.
    fn key_times(&self) -> Vec<f32> {
        let mut times: Vec<f32> = self
            .tracks
            .iter()
            .flat_map(|t| t.times.iter().copied())
            .collect();
        times.sort_by(f32::total_cmp);
        times.dedup();
        times
    }
}

/// What retargeting had to leave out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetargetReport {
    /// Source tracks whose bone has no counterpart in the mapping.
    pub unmapped_tracks: usize,
    /// Tracks whose key times and rotations disagreed in length.
    pub malformed_tracks: usize,
}

/// Samples a track's rotation at a time, holding the ends.
///
/// Nearest key rather than interpolating: every track being retargeted is
/// resampled onto key times drawn from the clip's own tracks, so a time between
/// two keys of *this* track is a time some other bone had a key at, and the
/// nearest value is the one that bone was actually holding.
fn sample(track: &RotationTrack, time: f32) -> Quat {
    if track.times.is_empty() {
        return Quat::IDENTITY;
    }
    let index = match track.times.binary_search_by(|t| t.total_cmp(&time)) {
        Ok(exact) => exact,
        Err(after) => {
            if after == 0 {
                0
            } else if after >= track.times.len() {
                track.times.len() - 1
            } else {
                let before = after - 1;
                if (time - track.times[before]).abs() <= (track.times[after] - time).abs() {
                    before
                } else {
                    after
                }
            }
        }
    };
    track
        .rotations
        .get(index)
        .copied()
        .unwrap_or(Quat::IDENTITY)
}

/// Moves a clip from the source skeleton onto the target skeleton.
///
/// `mapping` is source bone to target bone, as [`crate::automap::map_bones`]
/// produces. Bones the mapping does not cover keep their rest pose and are
/// counted in the report rather than guessed at.
pub fn retarget(
    source: &Skeleton,
    source_rest: &RestRotations,
    target: &Skeleton,
    target_rest: &RestRotations,
    mapping: &HashMap<usize, usize>,
    clip: &Clip,
) -> (Clip, RetargetReport) {
    let mut report = RetargetReport::default();
    let times = clip.key_times();

    // Source tracks by bone, skipping any whose keys do not line up with their
    // values -- a file can say otherwise.
    let mut by_bone: HashMap<usize, &RotationTrack> = HashMap::new();
    for track in &clip.tracks {
        if track.times.len() != track.rotations.len() {
            report.malformed_tracks += 1;
            continue;
        }
        if !mapping.contains_key(&track.bone) {
            report.unmapped_tracks += 1;
        }
        by_bone.insert(track.bone, track);
    }

    let source_rest_world = source_rest.world(source);
    let target_rest_world = target_rest.world(target);

    // Target bones in parent-before-child order, so a bone's parent already has
    // its animated world rotation when the bone is converted to local.
    let order = hierarchy_order(target);

    let mut out: HashMap<usize, Vec<Quat>> = HashMap::new();
    for &time in &times {
        // The source's animated world rotation per bone at this instant.
        let mut source_world = vec![Quat::IDENTITY; source.names.len()];
        for &bone in &hierarchy_order(source) {
            let local = by_bone
                .get(&bone)
                .map(|track| sample(track, time))
                .unwrap_or_else(|| {
                    source_rest
                        .local
                        .get(bone)
                        .copied()
                        .unwrap_or(Quat::IDENTITY)
                });
            let parent = source.parents.get(bone).copied().flatten();
            source_world[bone] = parent.map_or(Quat::IDENTITY, |p| source_world[p]) * local;
        }

        let mut target_world = vec![Quat::IDENTITY; target.names.len()];
        for &bone in &order {
            let from_source = mapping
                .iter()
                .find(|(_, &to)| to == bone)
                .map(|(&from, _)| from);

            let world = match from_source {
                Some(from) if by_bone.contains_key(&from) => {
                    // The motion the source made away from its own rest pose,
                    // applied to the target's rest pose.
                    let motion = source_world[from] * source_rest_world[from].inverse();
                    motion * target_rest_world[bone]
                }
                // Nothing drives this bone, so it keeps its **local** rest
                // rotation and follows whatever its parent is doing. Pinning it
                // to its rest *world* rotation instead makes it fight the
                // parent: an unmapped hand would hang in the air while the arm
                // swings. Found by a fixture built to animate a parent, which
                // every earlier one had left at identity.
                _ => {
                    let parent = target.parents.get(bone).copied().flatten();
                    let base = parent.map_or(Quat::IDENTITY, |p| target_world[p]);
                    base * target_rest
                        .local
                        .get(bone)
                        .copied()
                        .unwrap_or(Quat::IDENTITY)
                }
            };
            target_world[bone] = world;

            let parent = target.parents.get(bone).copied().flatten();
            let local = parent.map_or(world, |p| target_world[p].inverse() * world);
            out.entry(bone).or_default().push(local.normalize());
        }
    }

    let mut tracks: Vec<RotationTrack> = out
        .into_iter()
        .map(|(bone, rotations)| RotationTrack {
            bone,
            times: times.clone(),
            rotations,
        })
        .collect();
    tracks.sort_by_key(|t| t.bone);

    (
        Clip {
            name: clip.name.clone(),
            tracks,
        },
        report,
    )
}

/// Mirrors a clip left↔right across the sagittal (X) plane.
///
/// Each `_l`/`_r` bone's rotation track is moved to its partner, and every
/// rotation is reflected across X — `(x, y, z, w) → (x, -y, -z, w)` — so the
/// motion plays as its mirror image. A midline bone (no `_l`/`_r` suffix) keeps
/// its own track, reflected in place. Assumes a left/right-symmetric rest pose,
/// which the templates are (`calf_l` at +X mirrors `calf_r` at -X).
pub fn mirror_clip(clip: &Clip, names: &[String]) -> Clip {
    let partner = |bone: usize| -> usize {
        let Some(name) = names.get(bone) else {
            return bone;
        };
        let wanted = if let Some(base) = name.strip_suffix("_l") {
            format!("{base}_r")
        } else if let Some(base) = name.strip_suffix("_r") {
            format!("{base}_l")
        } else {
            return bone;
        };
        names.iter().position(|n| *n == wanted).unwrap_or(bone)
    };
    let tracks = clip
        .tracks
        .iter()
        .map(|track| RotationTrack {
            bone: partner(track.bone),
            times: track.times.clone(),
            rotations: track
                .rotations
                .iter()
                .map(|q| Quat::from_xyzw(q.x, -q.y, -q.z, q.w))
                .collect(),
        })
        .collect();
    Clip {
        name: clip.name.clone(),
        tracks,
    }
}

/// Widens or narrows the arms by `angle` radians (Mixamo's Character Arm-Space).
///
/// Pre-multiplies a spread rotation onto the `upperarm_l`/`upperarm_r` tracks;
/// the forearm and hand are its children, so they follow. A positive angle
/// raises the arms away from the body (wider), negative lowers them (narrower).
/// The right arm gets the opposite sign so the two stay symmetric.
pub fn spread_arms(clip: &Clip, names: &[String], angle: f32) -> Clip {
    if angle == 0.0 {
        return clip.clone();
    }
    let index = |name: &str| names.iter().position(|n| n == name);
    let left = index("upperarm_l");
    let right = index("upperarm_r");
    let spread_left = Quat::from_rotation_z(angle);
    let spread_right = Quat::from_rotation_z(-angle);
    let tracks = clip
        .tracks
        .iter()
        .map(|track| {
            let extra = if Some(track.bone) == left {
                Some(spread_left)
            } else if Some(track.bone) == right {
                Some(spread_right)
            } else {
                None
            };
            match extra {
                Some(spread) => RotationTrack {
                    bone: track.bone,
                    times: track.times.clone(),
                    rotations: track
                        .rotations
                        .iter()
                        .map(|q| (spread * *q).normalize())
                        .collect(),
                },
                None => track.clone(),
            }
        })
        .collect();
    Clip {
        name: clip.name.clone(),
        tracks,
    }
}

/// Bone indices with every parent before its children.
///
/// Bones caught in a parent cycle are placed last, so the walk terminates on a
/// skeleton a file could describe.
fn hierarchy_order(skeleton: &Skeleton) -> Vec<usize> {
    let mut depth = vec![usize::MAX; skeleton.names.len()];
    for index in 0..skeleton.names.len() {
        let mut chain = Vec::new();
        let mut cursor = Some(index);
        while let Some(current) = cursor {
            if depth[current] != usize::MAX || chain.contains(&current) {
                break;
            }
            chain.push(current);
            cursor = skeleton.parents.get(current).copied().flatten();
        }
        for &bone in chain.iter().rev() {
            let parent = skeleton.parents.get(bone).copied().flatten();
            depth[bone] = parent
                .map(|p| depth[p].saturating_add(1))
                .filter(|d| *d != usize::MAX)
                .unwrap_or(0);
        }
    }
    let mut order: Vec<usize> = (0..skeleton.names.len()).collect();
    order.sort_by_key(|&bone| depth[bone]);
    order
}

/// A skeleton's rest translations, one local offset per bone.
#[derive(Debug, Clone, PartialEq)]
pub struct RestTranslations {
    /// Local translation of each bone at rest.
    pub local: Vec<glam::Vec3>,
}

/// One bone's translation over time.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslationTrack {
    /// Index of the bone this drives.
    pub bone: usize,
    /// Key times, in seconds.
    pub times: Vec<f32>,
    /// One local translation per key.
    pub translations: Vec<glam::Vec3>,
}

/// How much longer the target is than the source, by overall height.
///
/// Root motion has to be scaled by this or a tall character takes a short
/// character's stride: the same clip on a 2.3x body would shuffle in place.
pub fn height_scale(source: &Skeleton, target: &Skeleton) -> f32 {
    let extent = |skeleton: &Skeleton| -> f32 {
        let ys: Vec<f32> = skeleton.positions.iter().map(|p| p.y).collect();
        let lo = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        (hi - lo).max(f32::EPSILON)
    };
    extent(target) / extent(source)
}

/// Moves translation tracks from the source skeleton onto the target.
///
/// # Why almost every bone comes out at its rest translation
///
/// A bone's local translation is its offset from its parent — its *length* —
/// not something an animation is supposed to change. Copying the source's would
/// rebuild the target with the source's proportions. Measured across the 87
/// clips in `human-base-animations.glb`: of **5,809** translation channels,
/// only **94 actually move**, and they belong to exactly two bones, `pelvis`
/// (80 clips) and `root` (14). Everything else is a constant equal to the rest
/// offset, written out by an exporter that emits full TRS whether or not it
/// varies.
///
/// So the same rule as rotations applies, and needs no special case for the
/// root: take the offset the source moved away from *its* rest translation,
/// scale it by [`height_scale`], and add it to the target's own rest. A bone
/// that never moves has a zero offset and lands exactly on the target's rest
/// translation, which is what preserves the target's proportions.
pub fn retarget_translations(
    source_rest: &RestTranslations,
    target_rest: &RestTranslations,
    mapping: &HashMap<usize, usize>,
    tracks: &[TranslationTrack],
    scale: f32,
) -> Vec<TranslationTrack> {
    let mut out = Vec::new();
    for track in tracks {
        if track.times.len() != track.translations.len() {
            continue;
        }
        let Some(&target_bone) = mapping.get(&track.bone) else {
            continue;
        };
        let Some(&source_rest_offset) = source_rest.local.get(track.bone) else {
            continue;
        };
        let Some(&target_rest_offset) = target_rest.local.get(target_bone) else {
            continue;
        };
        out.push(TranslationTrack {
            bone: target_bone,
            times: track.times.clone(),
            translations: track
                .translations
                .iter()
                .map(|&at| target_rest_offset + (at - source_rest_offset) * scale)
                .collect(),
        });
    }
    out.sort_by_key(|t| t.bone);
    out
}

#[cfg(test)]
mod mirror_tests {
    use super::{mirror_clip, spread_arms, Clip, RotationTrack};
    use glam::Quat;

    #[test]
    fn mirror_swaps_left_right_and_reflects_across_x() {
        let names: Vec<String> = ["hand_l", "hand_r", "spine"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let q = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9272).normalize();
        let clip = Clip {
            name: "wave".into(),
            tracks: vec![
                RotationTrack {
                    bone: 0,
                    times: vec![0.0],
                    rotations: vec![q],
                }, // hand_l
                RotationTrack {
                    bone: 2,
                    times: vec![0.0],
                    rotations: vec![q],
                }, // spine (midline)
            ],
        };

        let mirrored = mirror_clip(&clip, &names);

        // hand_l's track moved onto hand_r (bone 1), reflected (x, -y, -z, w).
        let hand = mirrored
            .tracks
            .iter()
            .find(|t| t.bone == 1)
            .expect("hand_r track");
        let m = hand.rotations[0];
        assert!((m.x - q.x).abs() < 1e-6);
        assert!((m.y + q.y).abs() < 1e-6);
        assert!((m.z + q.z).abs() < 1e-6);
        assert!((m.w - q.w).abs() < 1e-6);

        // The midline spine keeps its own bone, reflected in place.
        let spine = mirrored
            .tracks
            .iter()
            .find(|t| t.bone == 2)
            .expect("spine track");
        assert!((spine.rotations[0].y + q.y).abs() < 1e-6);
        // No track escaped onto hand_l here (only hand_l had a track to move away).
        assert!(mirrored.tracks.iter().all(|t| t.bone != 0));
    }

    #[test]
    fn spread_arms_rotates_the_upperarms_oppositely_and_leaves_others() {
        use glam::Quat;
        let names: Vec<String> = ["upperarm_l", "upperarm_r", "spine"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let clip = Clip {
            name: "t".into(),
            tracks: vec![
                RotationTrack {
                    bone: 0,
                    times: vec![0.0],
                    rotations: vec![Quat::IDENTITY],
                },
                RotationTrack {
                    bone: 1,
                    times: vec![0.0],
                    rotations: vec![Quat::IDENTITY],
                },
                RotationTrack {
                    bone: 2,
                    times: vec![0.0],
                    rotations: vec![Quat::IDENTITY],
                },
            ],
        };
        let angle = 0.3_f32;
        let out = spread_arms(&clip, &names, angle);
        let rot = |bone: usize| {
            out.tracks
                .iter()
                .find(|t| t.bone == bone)
                .unwrap()
                .rotations[0]
        };
        assert!(rot(0).angle_between(Quat::from_rotation_z(angle)) < 1e-5); // left +angle
        assert!(rot(1).angle_between(Quat::from_rotation_z(-angle)) < 1e-5); // right -angle
        assert!(rot(2).angle_between(Quat::IDENTITY) < 1e-6); // spine untouched
        assert_eq!(spread_arms(&clip, &names, 0.0), clip); // no-op at zero
    }
}
