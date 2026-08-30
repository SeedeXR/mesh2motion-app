//! Animation: curves and layers into clips of keyframe tracks.
//!
//! Ported from `legacy/src/lib/io/fbx/AnimationParser.ts`.
//!
//! FBX stores animation as four connected object kinds. An `AnimationCurve`
//! holds the keys for a single axis. An `AnimationCurveNode` groups the three
//! axes of one channel (`T`, `R`, `S`) and connects to the Model it drives. An
//! `AnimationLayer` collects curve nodes, and an `AnimationStack` is the clip.
//!
//! # Why the rotation path is the whole difficulty
//!
//! A rotation channel is stored as three independent Euler curves in degrees.
//! Turning them into quaternions means composing in the node's Euler order,
//! premultiplying its `PreRotation`, postmultiplying the inverse of its
//! `PostRotation`, and then unrolling sign flips so the track interpolates the
//! short way. Every one of those steps produces a smooth, unit-length,
//! entirely plausible track when it is wrong — so `tests/fbx_animation.rs`
//! diffs against the legacy's own output rather than checking properties.
//!
//! **43 of the 52 animated models in the reference rig carry a `PreRotation`.**
//!
//! # What the corpus does not exercise
//!
//! Measured over 8 Mixamo exports: no `PostRotation`, no scale or morph
//! channel, no curve node missing an axis, no two axes of a node disagreeing
//! in length, and no rotation step of 180° or more — so the sub-keyframe
//! interpolation below never fires on real data. Those paths are implemented
//! because other exporters produce them, and are covered synthetically.
//!
//! **Morph channels are the exception: they are counted, not parsed.** A
//! `DeformPercent` track drives a blend-shape weight, which needs the morph
//! target list from the geometry — and no geometry in this project carries
//! one yet. Rather than parse it into something with nowhere to go, the
//! channel is reported in [`AnimationReport::morph_channels_skipped`] so a
//! file that loses expression animation says so.

use crate::fbx::binary::FbxProperty;
use crate::fbx::dom::Scene;
use crate::fbx::model::ModelTree;
use crate::fbx::transform::EulerOrder;
use glam::{DQuat, DVec3};
use std::collections::HashMap;

/// Most keys one curve may carry.
///
/// The count comes from the file: a `KeyTime` array is bounded only by the
/// reader's 256 MB per-property inflate ceiling, which is 32 million keys, and
/// a few kilobytes of deflate can ask for a million. Even the linear matching below
/// would then allocate and walk millions of keys per axis per track. A minute
/// of animation at 120 fps is 7,200 keys; a quarter of a million is four hours
/// of it, and past that the file is not an animation.
const MAX_KEYS_PER_CURVE: usize = 262_144;

/// Most sub-keys one large rotation step may be split into.
///
/// The step count comes from a degree delta read straight out of the file, so
/// it is attacker-controlled: 1e7 degrees asks for 55,557 keys, and an
/// infinity saturates the float-to-int cast to `usize::MAX`, which is a loop
/// that never ends. 1024 sub-keys covers a rotation of 512 turns between two
/// adjacent keys, which is already far past anything an animation contains.
const MAX_SUBDIVISION: usize = 1024;

/// FBX counts time in units of 1/46186158000 of a second.
///
/// The constant is exact, not a rounding: it is 46186158000 = 2^4 × 3^4 × 5^3
/// × 7 × 11 × 13 × 17 × 19 × 23 × 29 × 31 × 37, chosen so every common frame
/// rate divides it evenly.
const FBX_TIME_UNIT: f64 = 46_186_158_000.0;

/// Converts an FBX key time to seconds.
pub fn seconds(fbx_time: i64) -> f64 {
    fbx_time as f64 / FBX_TIME_UNIT
}

/// What kind of value a track carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Three floats per key.
    Position,
    /// Four floats per key, xyzw.
    Quaternion,
    /// Three floats per key.
    Scale,
}

impl TrackKind {
    /// Floats per key.
    pub fn stride(self) -> usize {
        match self {
            Self::Quaternion => 4,
            _ => 3,
        }
    }
}

/// One animated property of one Model.
#[derive(Debug, Clone)]
pub struct Track {
    /// The Model this drives.
    pub model: i64,
    /// Which property.
    pub kind: TrackKind,
    /// Key times in seconds.
    pub times: Vec<f32>,
    /// `times.len() * kind.stride()` values.
    pub values: Vec<f32>,
}

/// One animation clip, from an `AnimationStack`.
#[derive(Debug, Clone)]
pub struct Clip {
    /// The stack's name, e.g. `mixamo.com`.
    pub name: String,
    /// Longest track end time, in seconds. Zero when there are no tracks.
    pub duration: f64,
    /// Tracks in layer-connection order.
    pub tracks: Vec<Track>,
}

/// What parsing had to drop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnimationReport {
    /// Curves connected to nothing, or to a node that is not a curve node.
    ///
    /// **153 of the reference rig's 315 curves** are exactly this — an
    /// ordinary property of Mixamo exports, not a defect, which is why it is
    /// counted rather than warned about.
    pub unattached_curves: usize,
    /// Curve nodes in a layer that name no Model, so they drive nothing.
    pub curve_nodes_without_model: usize,
    /// Curve nodes whose Model is not in the scene.
    pub curve_nodes_with_missing_model: usize,
    /// Channels dropped for having no curve on any axis.
    pub empty_channels: usize,
    /// Morph (`DeformPercent`) channels, which this does not yet parse.
    ///
    /// Counted rather than ignored: a file whose facial animation vanished
    /// would otherwise load looking complete.
    pub morph_channels_skipped: usize,
    /// Curves truncated at [`MAX_KEYS_PER_CURVE`].
    pub curves_over_key_limit: usize,
    /// Keys dropped for holding a value that is not finite.
    ///
    /// A NaN or infinite Euler angle cannot become a quaternion — it becomes a
    /// NaN one, which spreads into every vertex the bone touches and surfaces
    /// as the mesh disappearing, a long way from the cause.
    pub non_finite_keys: usize,
    /// Rotation channels left with no usable key at all.
    pub channels_without_a_usable_key: usize,
    /// Rotation steps whose subdivision hit [`MAX_SUBDIVISION`].
    ///
    /// The step count comes from a degree delta in the file, so it is bounded
    /// rather than trusted.
    pub subdivisions_capped: usize,
    /// Animation layers beyond the first in a stack, which are not applied.
    ///
    /// FBX allows a stack to blend several layers; the legacy takes the first
    /// and warns. Applying them all would double-transform every bone the
    /// layers share, so this matches the legacy and counts what it skipped.
    pub extra_layers_skipped: usize,
    /// Curves whose `KeyTime` and `KeyValueFloat` arrays disagreed in length.
    ///
    /// Truncated to the shorter, so a key always has both a time and a value.
    /// Without this a short value array indexes out of bounds while sampling —
    /// a panic reached straight from file content.
    pub mismatched_curve_arrays: usize,
}

/// One axis of one channel.
#[derive(Debug, Clone, Default)]
struct Curve {
    times: Vec<f64>,
    values: Vec<f64>,
}

/// The three axes of one channel of one Model.
#[derive(Debug, Clone, Default)]
struct CurveNode {
    channel: Channel,
    axes: [Option<Curve>; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Channel {
    Translation,
    #[default]
    Rotation,
    Scale,
}

impl CurveNode {
    fn is_empty(&self) -> bool {
        self.axes.iter().all(Option::is_none)
    }
}

/// Reads every clip in the scene.
pub fn parse_all(scene: &Scene, models: &ModelTree) -> (Vec<Clip>, AnimationReport) {
    let mut report = AnimationReport::default();
    let nodes = curve_nodes(scene, &mut report);

    // Stack order is the clip order, and it is ascending object id.
    //
    // That matches the legacy for every id below 2^32, because JavaScript
    // enumerates integer-like object keys in that range numerically. Above it
    // the key stops being an array index and V8 falls back to insertion order,
    // so a file with very large object ids would give the legacy file order
    // and this port id order. Ascending id is kept regardless: it is the
    // deterministic choice, and the alternative depends on how the file
    // happened to be written. The reference rig's stacks are 477116448 and
    // 485299280, both well under the limit, so no fixture can see this.
    let clips: Vec<Clip> = scene
        .objects_of_kind("AnimationStack")
        .into_iter()
        .map(|stack| {
            // Only the FIRST layer, matching the legacy. FBX lets a stack
            // blend several layers with per-layer weights and blend modes;
            // applying them all here would emit two tracks for the same bone
            // and property in one clip, which a player either double-applies
            // or resolves arbitrarily. Skipped layers are counted rather than
            // silently ignored.
            let layers = scene.children_of(stack.id, Some("AnimationLayer"));
            report.extra_layers_skipped += layers.len().saturating_sub(1);
            let tracks: Vec<Track> = layers
                .first()
                .map(|&layer| layer_tracks(scene, models, &nodes, layer, &mut report))
                .unwrap_or_default();
            // three.js passes duration -1 and lets AnimationClip.resetDuration
            // take the longest track's last key; an empty clip gets zero.
            let duration = tracks
                .iter()
                .filter_map(|t| t.times.last())
                .fold(0.0f64, |acc, &t| acc.max(f64::from(t)));
            Clip {
                name: stack.name.clone(),
                duration,
                tracks,
            }
        })
        .collect();
    (clips, report)
}

/// Reads the curve nodes and attaches each curve to its axis.
fn curve_nodes(scene: &Scene, report: &mut AnimationReport) -> HashMap<i64, CurveNode> {
    let mut nodes: HashMap<i64, CurveNode> = scene
        .objects_of_kind("AnimationCurveNode")
        .into_iter()
        .filter_map(|o| {
            // The name is the channel: `T`, `R`, `S`. The legacy tests it with
            // /S|R|T|DeformPercent/, so a name merely CONTAINING one of those
            // letters qualifies there too.
            // Checked before T/R/S: `DeformPercent` contains neither, but
            // naming it explicitly keeps the intent readable and survives a
            // channel name that happens to share a letter.
            if o.name.contains("DeformPercent") {
                report.morph_channels_skipped += 1;
                return None;
            }
            let channel = if o.name.contains('T') {
                Channel::Translation
            } else if o.name.contains('R') {
                Channel::Rotation
            } else if o.name.contains('S') {
                Channel::Scale
            } else {
                return None;
            };
            Some((
                o.id,
                CurveNode {
                    channel,
                    axes: Default::default(),
                },
            ))
        })
        .collect();

    for curve in scene.objects_of_kind("AnimationCurve") {
        let times: Vec<f64> = curve
            .node
            .child("KeyTime")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_i64_vec)
            .unwrap_or_default()
            .into_iter()
            .map(seconds)
            .collect();
        let mut values: Vec<f64> = curve
            .node
            .child("KeyValueFloat")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_f64_vec)
            .unwrap_or_default();

        // A key needs both a time and a value. Keeping the leading pairs is
        // better than dropping the curve, and it keeps every later index in
        // range — sampling walks the time array and reads the value array at
        // the same position.
        let mut times = times;
        if times.len() != values.len() {
            report.mismatched_curve_arrays += 1;
            let keep = times.len().min(values.len());
            times.truncate(keep);
            values.truncate(keep);
        }
        // Cap here, before anything downstream is sized from the key count.
        if times.len() > MAX_KEYS_PER_CURVE {
            report.curves_over_key_limit += 1;
            times.truncate(MAX_KEYS_PER_CURVE);
            values.truncate(MAX_KEYS_PER_CURVE);
        }

        // The connection's property name carries the axis: `d|X`, `d|Y`, `d|Z`.
        let parent = scene.links.get(&curve.id).and_then(|l| l.parents.first());
        let axis = parent.and_then(|p| p.property.as_deref()).and_then(|p| {
            if p.contains('X') {
                Some(0)
            } else if p.contains('Y') {
                Some(1)
            } else if p.contains('Z') {
                Some(2)
            } else {
                None
            }
        });
        match (parent, axis) {
            (Some(parent), Some(axis)) => match nodes.get_mut(&parent.id) {
                Some(node) => node.axes[axis] = Some(Curve { times, values }),
                None => report.unattached_curves += 1,
            },
            _ => report.unattached_curves += 1,
        }
    }
    nodes
}

/// Builds the tracks for one layer, in connection order.
fn layer_tracks(
    scene: &Scene,
    models: &ModelTree,
    nodes: &HashMap<i64, CurveNode>,
    layer: i64,
    report: &mut AnimationReport,
) -> Vec<Track> {
    let mut tracks = Vec::new();
    for child in scene.children_of(layer, Some("AnimationCurveNode")) {
        let Some(node) = nodes.get(&child) else {
            continue;
        };
        if node.is_empty() {
            report.empty_channels += 1;
            continue;
        }
        // The Model is the first parent reached through a *property*
        // connection; the other parent is the layer, which is object-to-object
        // and so has no property name.
        let Some(model_id) = scene
            .links
            .get(&child)
            .and_then(|l| l.parents.iter().find(|p| p.property.is_some()))
            .map(|p| p.id)
        else {
            report.curve_nodes_without_model += 1;
            continue;
        };
        let Some(model) = models.get(model_id) else {
            report.curve_nodes_with_missing_model += 1;
            continue;
        };

        // Each curve node becomes its own track: the legacy keys its per-layer
        // table by the child's index, so a Model's T and R channels are
        // separate entries rather than being merged.
        let track = match node.channel {
            Channel::Translation => {
                let (_, _, translation) = model.local.to_scale_rotation_translation();
                Some(vector_track(
                    model_id,
                    TrackKind::Position,
                    node,
                    translation,
                    report,
                ))
            }
            Channel::Scale => {
                let (scale, _, _) = model.local.to_scale_rotation_translation();
                Some(vector_track(
                    model_id,
                    TrackKind::Scale,
                    node,
                    scale,
                    report,
                ))
            }
            Channel::Rotation => rotation_track(model_id, node, model, report),
        };
        if let Some(track) = track {
            tracks.push(track);
        }
    }
    tracks
}

/// Every key time across the three axes, sorted, deduplicated.
fn merged_times(node: &CurveNode) -> Vec<f64> {
    let mut times: Vec<f64> = node
        .axes
        .iter()
        .flatten()
        .flat_map(|c| c.times.iter().copied())
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    times.dedup();
    times
}

/// A position or scale track.
///
/// Where an axis has no key at a given time, the previous key's value carries
/// forward — starting from the Model's own value for that axis.
fn vector_track(
    model: i64,
    kind: TrackKind,
    node: &CurveNode,
    initial: DVec3,
    report: &mut AnimationReport,
) -> Track {
    let times = merged_times(node);
    let mut previous = [initial.x, initial.y, initial.z];
    let mut values = Vec::with_capacity(times.len() * 3);
    let mut non_finite = 0usize;

    // One forward cursor per axis rather than a search per key. The merged
    // array is the sorted union of all three axes' times, so each axis's own
    // times appear in it in order and a single pass finds them all — turning
    // what was quadratic in the key count into linear. At the reference rig's
    // 7844 keys the difference was immaterial, but the count comes from the
    // file, and a quadratic walk over a million keys is a hang, not a delay.
    let mut cursor = [0usize; 3];
    for &time in &times {
        for ((curve, previous), cursor) in node
            .axes
            .iter()
            .zip(previous.iter_mut())
            .zip(cursor.iter_mut())
        {
            if let Some(curve) = curve {
                // Skip any of this curve's keys that the merged array has
                // already passed. That cannot happen while the curve's times
                // are sorted — and they are, in every real file — but a
                // malformed one must not make the cursor stick.
                while *cursor < curve.times.len() && curve.times[*cursor] < time {
                    *cursor += 1;
                }
                // Exact equality is right here: the merged array is built from
                // these very floats, so a key either is in it or is not.
                if curve.times.get(*cursor) == Some(&time) {
                    let i = *cursor;
                    *cursor += 1;
                    match curve.values.get(i) {
                        // A non-finite position or scale would place the bone
                        // nowhere and take the mesh with it. Hold the last
                        // good value instead, and say that a key was lost.
                        Some(&v) if v.is_finite() => *previous = v,
                        Some(_) => non_finite += 1,
                        None => {}
                    }
                }
            }
            values.push(*previous as f32);
        }
    }
    report.non_finite_keys += non_finite;
    Track {
        model,
        kind,
        times: times.into_iter().map(|t| t as f32).collect(),
        values,
    }
}

/// Samples a curve at `time`, clamping outside its range.
///
/// `from` is a forward cursor: the caller walks target times in ascending
/// order, so each call resumes where the last left off instead of rescanning.
/// Without that this is O(targets x keys), and both counts come from the file.
fn sample(curve: &Curve, time: f64, from: &mut usize) -> f64 {
    let (times, values) = (&curve.times, &curve.values);
    let Some(&first) = values.first() else {
        return 0.0;
    };
    if time <= times[0] {
        return first;
    }
    if time >= *times.last().unwrap_or(&time) {
        return *values.last().unwrap_or(&first);
    }
    // Advance to the last key at or before `time`. A malformed curve whose
    // times are not sorted would leave the cursor early rather than looping.
    while *from + 1 < times.len() && times[*from + 1] <= time {
        *from += 1;
    }
    let i = *from;
    if times[i] == time {
        return values[i];
    }
    let Some(&next) = times.get(i + 1) else {
        return values[i];
    };
    let span = next - times[i];
    let alpha = if span == 0.0 {
        0.0
    } else {
        (time - times[i]) / span
    };
    let (a, b) = (values[i], values.get(i + 1).copied().unwrap_or(values[i]));
    a * (1.0 - alpha) + b * alpha
}

/// Puts one axis onto the merged time array.
///
/// An axis with no curve holds the Model's own resting value throughout —
/// which is why a rotation track needs `Lcl_Rotation` and not just the curves.
fn synchronise(curve: Option<&Curve>, times: &[f64], initial: f64) -> Vec<f64> {
    let Some(curve) = curve else {
        return vec![initial; times.len()];
    };
    // Compare the TIMES, not just their count. The legacy checks only
    // `curve.times.length === targetTimes.length` and assumes the times match,
    // which a curve with a duplicated `KeyTime` breaks: its values outnumber
    // its distinct times, so it can match the deduplicated merged length while
    // its keys sit elsewhere, and every value lands one slot late with the
    // last invented. The track then looks complete and reads wrong.
    if curve.times == times && curve.values.len() == times.len() {
        return curve.values.clone();
    }
    let mut cursor = 0usize;
    times
        .iter()
        .map(|&t| sample(curve, t, &mut cursor))
        .collect()
}

/// Euler angles in degrees to a quaternion, in the given order.
///
/// Built as the product of the three axis quaternions in the order's own
/// sequence, which is what three.js's `setFromEuler` expands to — including
/// its choice of sign, which matters because the track stores the components
/// and not just the rotation they represent.
fn euler_quat(degrees: [f64; 3], order: EulerOrder) -> DQuat {
    let [a, b, c] = order.axis_quats(degrees);
    a * b * c
}

/// Builds a quaternion track from three Euler curves.
fn rotation_track(
    model: i64,
    node: &CurveNode,
    model_data: &crate::fbx::model::Model,
    report: &mut AnimationReport,
) -> Option<Track> {
    let times = merged_times(node);
    if times.is_empty() {
        return None;
    }

    let initial = model_data.transform.rotation;
    let axes = [
        synchronise(node.axes[0].as_ref(), &times, initial.x),
        synchronise(node.axes[1].as_ref(), &times, initial.y),
        synchronise(node.axes[2].as_ref(), &times, initial.z),
    ];
    let order = model_data.transform.euler_order;

    // Pre- and post-rotation always use the DEFAULT Euler order, even when the
    // node declares another — the same rule the transform pipeline follows.
    let pre = (model_data.transform.pre_rotation != DVec3::ZERO).then(|| {
        euler_quat(
            model_data.transform.pre_rotation.to_array(),
            EulerOrder::default(),
        )
    });
    let post = (model_data.transform.post_rotation != DVec3::ZERO).then(|| {
        euler_quat(
            model_data.transform.post_rotation.to_array(),
            EulerOrder::default(),
        )
        .inverse()
    });

    let key = |i: usize| [axes[0][i], axes[1][i], axes[2][i]];

    // A key holding a non-finite value cannot become a quaternion — it becomes
    // a NaN one, which propagates into every vertex the bone touches and shows
    // up as the mesh vanishing, far from here. Drop such keys and say so.
    // The legacy skips them mid-track but emits key 0 unconditionally, so a
    // file whose first key is NaN poisons the track there.
    let usable: Vec<usize> = (0..times.len())
        .filter(|&i| key(i).iter().all(|v| v.is_finite()))
        .collect();
    report.non_finite_keys += times.len() - usable.len();
    let Some(&first) = usable.first() else {
        report.channels_without_a_usable_key += 1;
        return None;
    };

    let mut out_times: Vec<f64> = Vec::with_capacity(usable.len());
    let mut quats: Vec<DQuat> = Vec::with_capacity(usable.len());

    out_times.push(times[first]);
    quats.push(euler_quat(key(first), order));

    for w in usable.windows(2) {
        let (prev_i, i) = (w[0], w[1]);
        let (previous, current) = (key(prev_i), key(i));
        let span = (0..3)
            .map(|a| (current[a] - previous[a]).abs())
            .fold(0.0f64, f64::max);

        // A quaternion cannot represent a turn of more than half a revolution
        // as a single step, so an Euler pair that far apart has to be split or
        // the track interpolates the wrong way round.
        //
        // Two deliberate divergences from the legacy here, both unreachable on
        // any file in the corpus (measured: zero steps of 180° or more):
        // it slerps and then round-trips through Euler, which is lossy at
        // gimbal lock, where this stays in quaternion space; and its
        // subdivision loop stops before t = 1, dropping the final key of a
        // large step when no key follows it.
        if span >= 180.0 {
            let from = euler_quat(previous, order);
            let to = euler_quat(current, order);
            // `span` is finite here — the key filter above dropped any key
            // holding a non-finite value — but clamp regardless, because the
            // cast saturates rather than wrapping and one absurd key would
            // otherwise allocate without bound.
            let wanted = (span / 180.0).ceil();
            let steps = if wanted.is_finite() {
                (wanted as usize).clamp(1, MAX_SUBDIVISION)
            } else {
                MAX_SUBDIVISION
            };
            if steps == MAX_SUBDIVISION {
                report.subdivisions_capped += 1;
            }
            for step in 1..=steps {
                let t = step as f64 / steps as f64;
                out_times.push(times[prev_i] + (times[i] - times[prev_i]) * t);
                quats.push(from.slerp(to, t));
            }
        } else {
            out_times.push(times[i]);
            quats.push(euler_quat(current, order));
        }
    }

    let mut values = Vec::with_capacity(quats.len() * 4);
    let mut previous: Option<DQuat> = None;
    for mut q in quats {
        if let Some(pre) = pre {
            q = pre * q;
        }
        if let Some(post) = post {
            q *= post;
        }
        // Unroll: neighbouring keys must stay on the same hemisphere, or the
        // renderer interpolates the long way round and the bone spins.
        if let Some(p) = previous {
            if p.dot(q) < 0.0 {
                q = -q;
            }
        }
        previous = Some(q);
        values.extend([q.x as f32, q.y as f32, q.z as f32, q.w as f32]);
    }

    Some(Track {
        model,
        kind: TrackKind::Quaternion,
        times: out_times.into_iter().map(|t| t as f32).collect(),
        values,
    })
}
