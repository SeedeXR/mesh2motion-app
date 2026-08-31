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
