//! Animation clips, diffed key-for-key against the legacy loader.
//!
//! `legacy/bench/dump-animation-fixtures.ts` runs the legacy's own `FBXLoader`
//! headless over the reference rig and records every clip's tracks. A
//! quaternion track is the product of Euler-order composition, a `PreRotation`
//! premultiply and a sign-unroll pass, and each of those yields a smooth,
//! unit-length, plausible track when wrong — so this compares values, not
//! properties.

use m2m_io::fbx::animation::{self, Clip, TrackKind};
use m2m_io::fbx::{binary, dom::Scene, model};

const MIXAMO: &[u8] =
    include_bytes!("../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx");
const FIXTURE: &[u8] = include_bytes!("fixtures/fbx-anim.bin");
const NAMES: &str = include_str!("fixtures/fbx-anim-names.txt");

struct ExpectedTrack {
    name: String,
    kind: TrackKind,
    times: Vec<f64>,
    values: Vec<f64>,
}

struct ExpectedClip {
    name: String,
    duration: f64,
    tracks: Vec<ExpectedTrack>,
}

fn expected() -> Vec<ExpectedClip> {
    let count = u32::from_le_bytes(FIXTURE[0..4].try_into().expect("header")) as usize;
    let body = &FIXTURE[8..];
    let f = |i: usize| f64::from_le_bytes(body[i * 8..i * 8 + 8].try_into().expect("f64"));

    let mut names = NAMES.lines();
    let mut cursor = 0usize;
    let mut clips = Vec::with_capacity(count);
    for _ in 0..count {
        let name = names
            .next()
            .and_then(|l| l.strip_prefix("clip "))
            .expect("clip name")
            .to_string();
        let duration = f(cursor);
        let track_count = f(cursor + 1) as usize;
        cursor += 2;
        let mut tracks = Vec::with_capacity(track_count);
        for _ in 0..track_count {
            let track_name = names
                .next()
                .and_then(|l| l.strip_prefix("track "))
                .expect("track name")
                .to_string();
            let kind = match f(cursor) as usize {
                0 => TrackKind::Position,
                1 => TrackKind::Quaternion,
                _ => TrackKind::Scale,
            };
            let key_count = f(cursor + 1) as usize;
            let value_count = f(cursor + 2) as usize;
            cursor += 3;
            let times: Vec<f64> = (0..key_count).map(|i| f(cursor + i)).collect();
            cursor += key_count;
            let values: Vec<f64> = (0..value_count).map(|i| f(cursor + i)).collect();
            cursor += value_count;
            tracks.push(ExpectedTrack {
                name: track_name,
                kind,
                times,
                values,
            });
        }
        clips.push(ExpectedClip {
            name,
            duration,
            tracks,
        });
    }
    assert_eq!(cursor * 8, body.len(), "fixture not fully consumed");
    clips
}

fn parsed() -> (Vec<Clip>, model::ModelTree, animation::AnimationReport) {
    let scene = Scene::from_document(binary::parse(MIXAMO).expect("parses"));
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);
    (clips, models, report)
}

#[test]
fn every_clip_and_key_matches_the_legacy() {
    let (clips, models, _) = parsed();
    let expected = expected();

    assert_eq!(clips.len(), expected.len(), "clip count");
    // Not a vacuous loop: the rig carries two clips and 7844 keys.
    let total_keys: usize = expected
        .iter()
        .flat_map(|c| c.tracks.iter())
        .map(|t| t.times.len())
        .sum();
    assert_eq!(total_keys, 7844, "keys in the fixture");

    let mut worst_time = 0.0f64;
    let mut worst_value = 0.0f64;
    let mut worst_where = String::new();

    for (clip, want) in clips.iter().zip(expected.iter()) {
        assert_eq!(clip.name, want.name, "clip name");
        assert!(
            (clip.duration - want.duration).abs() < 1e-6,
            "{}: duration {} vs {}",
            clip.name,
            clip.duration,
            want.duration
        );
        assert_eq!(clip.tracks.len(), want.tracks.len(), "{} tracks", clip.name);

        for (track, want) in clip.tracks.iter().zip(want.tracks.iter()) {
            // three.js strips ':' from names; the FBX and this port keep it.
            let model = models.get(track.model).expect("model");
            let (legacy_model, suffix) = want.name.split_once('.').expect("dotted track name");
            assert_eq!(
                model.name.replace(':', ""),
                legacy_model,
                "{} track targets a different model",
                clip.name
            );
            assert_eq!(
                match track.kind {
                    TrackKind::Position => "position",
                    TrackKind::Quaternion => "quaternion",
                    TrackKind::Scale => "scale",
                },
                suffix,
                "{} track kind",
                want.name
            );
            assert_eq!(track.kind, want.kind);
            assert_eq!(track.times.len(), want.times.len(), "{} keys", want.name);
            assert_eq!(
                track.values.len(),
                want.values.len(),
                "{} values",
                want.name
            );
            assert_eq!(
                track.values.len(),
                track.times.len() * track.kind.stride(),
                "{} stride",
                want.name
            );

            for (i, (&got, &wanted)) in track.times.iter().zip(want.times.iter()).enumerate() {
                let d = (f64::from(got) - wanted).abs();
                if d > worst_time {
                    worst_time = d;
                    worst_where = format!("{} time[{i}]", want.name);
                }
                // Measured worst deviation is EXACTLY zero: both sides round
                // the same f64 to f32, so any real difference is at least one
                // f32 ULP (~5e-7 at these magnitudes). A loose 1e-6 here let a
                // 4e-5 relative error in the FBX time constant pass unnoticed.
                assert!(d < 1e-9, "{} time[{i}]: {got} vs {wanted}", want.name);
            }
            for (i, (&got, &wanted)) in track.values.iter().zip(want.values.iter()).enumerate() {
                let d = (f64::from(got) - wanted).abs();
                if d > worst_value {
                    worst_value = d;
                    worst_where = format!("{} value[{i}]", want.name);
                }
                // Positions run to ~100cm and quaternion components to 1, and
                // three.js stores both as f32, so this is f32 rounding.
                assert!(d < 1e-4, "{} value[{i}]: {got} vs {wanted}", want.name);
            }
        }
    }
    eprintln!("worst time {worst_time:e}, worst value {worst_value:e} (at {worst_where})");
}

#[test]
fn the_clips_are_what_the_file_actually_contains() {
    // Independent of the fixture: if the dump were regenerated from a broken
    // loader, every assertion above would still hold.
    let (clips, models, report) = parsed();

    // Clip order is ascending stack id, not alphabetical: stack 477116448 is
    // `mixamo.com` and 485299280 is `Take 001`.
    let names: Vec<&str> = clips.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["mixamo.com", "Take 001"],
        "clips in stack-id order"
    );

    // 'Take 001' is a real stack whose layer has no curve nodes at all. An
    // empty clip must survive as an empty clip, not vanish and not error.
    let empty = &clips[1];
    assert!(empty.tracks.is_empty());
    assert_eq!(empty.duration, 0.0, "no tracks means no duration");

    let main = &clips[0];
    assert_eq!(main.tracks.len(), 53);
    assert!(
        (main.duration - 4.9).abs() < 0.01,
        "duration {}",
        main.duration
    );

    // One position track (the hips) and 52 rotations: Mixamo animates root
    // translation only, everything else is joint rotation.
    let positions = main
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Position)
        .count();
    assert_eq!(positions, 1, "only the root translates");
    assert_eq!(
        main.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Quaternion)
            .count(),
        52
    );
    let hips = main
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Position)
        .expect("position track");
    assert_eq!(
        models.get(hips.model).expect("model").name,
        "mixamorig:Hips"
    );

    // Every quaternion is unit length: the one property that must hold however
    // the composition is ordered, and it fails loudly if pre-rotation is
    // applied as a raw Euler triple rather than a quaternion.
    for track in main
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Quaternion)
    {
        for key in track.values.chunks_exact(4) {
            let length = (key
                .iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>())
            .sqrt();
            assert!(
                (length - 1.0).abs() < 1e-5,
                "non-unit quaternion {length} in {}",
                models.get(track.model).expect("model").name
            );
        }
    }

    // 153 of the 315 curves belong to no curve node. That is ordinary for a
    // Mixamo export, and pinning it means a change that started silently
    // dropping ATTACHED curves would show up here.
    assert_eq!(report.unattached_curves, 153);
    assert_eq!(report.curve_nodes_without_model, 0);
    assert_eq!(report.curve_nodes_with_missing_model, 0);
}

/// A document with the given `Objects` bodies and `Connections` lines.
fn anim_doc(objects: &str, connections: &str) -> Scene {
    let text =
        format!("FBXVersion: 7400\nObjects:  {{\n{objects}}}\nConnections:  {{\n{connections}}}\n");
    Scene::from_document(m2m_io::fbx::text::parse(&text).expect("ascii parses"))
}

/// One `AnimationCurve` node. Times are in FBX ticks, values in the channel's
/// own units (degrees for a rotation).
fn curve(id: i64, times: &[i64], values: &[f64]) -> String {
    let t: Vec<String> = times.iter().map(|v| v.to_string()).collect();
    let v: Vec<String> = values.iter().map(|v| format!("{v}")).collect();
    format!(
        "\tAnimationCurve: {id}, \"AnimCurve::\", \"\" {{\n\t\tKeyTime: *{} {{\n\t\t\ta: {}\n\t\t}}\n\t\tKeyValueFloat: *{} {{\n\t\t\ta: {}\n\t\t}}\n\t}}\n",
        times.len(),
        t.join(","),
        values.len(),
        v.join(",")
    )
}

/// One FBX tick per second, so key times read as whole seconds.
const TICK: i64 = 46_186_158_000;

/// A stack, a layer, and one `R` curve node driving model 10, whose three axes
/// are the given degree sequences at the given tick times.
fn rotating_bone(model_props: &str, times: &[i64], xyz: [&[f64]; 3]) -> Scene {
    let mut objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t\tProperties70:  {{\n{model_props}\t\t}}\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::clip\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::Base\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n"
    );
    let mut connections = String::from(
        "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\tC: \"OP\",300,10,\"Lcl Rotation\"\n",
    );
    for (axis, values) in xyz.iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        let id = 400 + axis as i64;
        objects.push_str(&curve(id, times, values));
        let letter = ["X", "Y", "Z"][axis];
        connections.push_str(&format!("\tC: \"OP\",{id},300,\"d|{letter}\"\n"));
    }
    anim_doc(&objects, &connections)
}

fn only_clip(scene: &Scene) -> (Clip, model::ModelTree) {
    let models = model::parse_all(scene);
    let (clips, _) = animation::parse_all(scene, &models);
    assert_eq!(clips.len(), 1, "one stack");
    (clips.into_iter().next().expect("clip"), models)
}

#[test]
fn neighbouring_keys_are_unrolled_onto_the_same_hemisphere() {
    // Measured: not one of the reference rig's 7644 adjacent key pairs needs
    // this, so the fixture cannot see it. Without the unroll a renderer
    // slerping between q and -q takes the long way round, and the bone spins
    // through most of a turn between two keys that are barely apart.
    //
    // The pair below was found by search, not derived: from (-180,-150,-180)
    // to (-30,-30,-30) every axis moves less than 180 -- keeping the
    // sub-division path out of it -- while the composed quaternions have a
    // dot of -0.90. An earlier version of this test used (0,0,0) to
    // (170,170,170), which cannot flip at all: no triple with every axis under
    // 180 has a negative dot with the identity in this Euler order.
    let (clip, _) = only_clip(&rotating_bone(
        "",
        &[0, TICK],
        [&[-180.0, -30.0], &[-150.0, -30.0], &[-180.0, -30.0]],
    ));
    let track = &clip.tracks[0];
    assert_eq!(track.times.len(), 2, "no sub-division should have occurred");

    let q: Vec<[f32; 4]> = track
        .values
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    let dot: f32 = (0..4).map(|i| q[0][i] * q[1][i]).sum();
    assert!(
        dot >= 0.0,
        "keys are on opposite hemispheres (dot {dot}) -- the unroll did not fire"
    );

    // Premise: without the unroll this pair really does flip. Recomputing the
    // raw composition here means the test cannot pass by the flip never
    // arising, which is exactly how the earlier version was vacuous.
    use m2m_io::fbx::transform::EulerOrder;
    let raw = |d: [f64; 3]| {
        let [a, b, c] = EulerOrder::default().axis_quats(d);
        a * b * c
    };
    let raw_dot = raw([-180.0, -150.0, -180.0]).dot(raw([-30.0, -30.0, -30.0]));
    assert!(
        raw_dot < -0.5,
        "premise broken: the raw pair no longer flips (dot {raw_dot})"
    );
}

#[test]
fn the_models_euler_order_is_used_for_rotation_keys() {
    // No file in the corpus sets RotationOrder, so every animated model uses
    // order 0 and the fixture cannot tell whether the field is read at all.
    let order_2 = "\t\t\tP: \"RotationOrder\", \"enum\", \"\", \"\",2\n";
    let keys: [&[f64]; 3] = [&[0.0, 30.0], &[0.0, 40.0], &[0.0, 50.0]];

    let (default_order, _) = only_clip(&rotating_bone("", &[0, TICK], keys));
    let (explicit, _) = only_clip(&rotating_bone(order_2, &[0, TICK], keys));

    let a = &default_order.tracks[0].values;
    let b = &explicit.tracks[0].values;
    let deviation = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        deviation > 0.01,
        "changing RotationOrder changed nothing (max {deviation})"
    );
}

#[test]
fn an_axis_with_no_curve_holds_the_models_own_rotation() {
    // Every curve node in the corpus carries all three axes, so this fill-in
    // never runs there. When it does run and is wrong, the bone rests at zero
    // on the missing axes instead of its authored pose.
    let props = "\t\t\tP: \"Lcl Rotation\", \"Lcl Rotation\", \"\", \"A\",11,22,33\n";
    // Only X is animated.
    let (clip, _) = only_clip(&rotating_bone(props, &[0, TICK], [&[0.0, 60.0], &[], &[]]));
    let track = &clip.tracks[0];
    assert_eq!(track.times.len(), 2);

    // Rebuild what the Y and Z angles must have been: compare against the
    // quaternion for (x, 22, 33) at each key, computed independently.
    use m2m_io::fbx::transform::EulerOrder;
    for (i, x) in [0.0f64, 60.0].into_iter().enumerate() {
        let [qa, qb, qc] = EulerOrder::default().axis_quats([x, 22.0, 33.0]);
        let want = qa * qb * qc;
        let got = &track.values[i * 4..i * 4 + 4];
        let dev = [want.x, want.y, want.z, want.w]
            .iter()
            .zip(got.iter())
            .map(|(a, b)| (a - f64::from(*b)).abs())
            .fold(0.0f64, f64::max);
        assert!(
            dev < 1e-6,
            "key {i} deviates by {dev}; Y/Z were not filled from Lcl_Rotation"
        );
    }
}

#[test]
fn clip_duration_is_the_longest_track_not_the_first() {
    // All 53 tracks in the reference rig end at 4.9s, so the fixture cannot
    // distinguish "longest" from "first".
    let objects = format!(
        "\tModel: 10, \"Model::a\", \"LimbNode\" {{\n\t}}\n\
         \tModel: 11, \"Model::b\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::clip\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::Base\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 310, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{}{}{}{}{}{}",
        curve(400, &[0, TICK], &[0.0, 10.0]),
        curve(401, &[0, TICK], &[0.0, 10.0]),
        curve(402, &[0, TICK], &[0.0, 10.0]),
        // The SECOND node runs four times as long.
        curve(410, &[0, 4 * TICK], &[0.0, 10.0]),
        curve(411, &[0, 4 * TICK], &[0.0, 10.0]),
        curve(412, &[0, 4 * TICK], &[0.0, 10.0]),
    );
    let connections = "\tC: \"OO\",200,100\n\
         \tC: \"OO\",300,200\n\tC: \"OP\",300,10,\"Lcl Rotation\"\n\
         \tC: \"OO\",310,200\n\tC: \"OP\",310,11,\"Lcl Rotation\"\n\
         \tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n\
         \tC: \"OP\",410,310,\"d|X\"\n\tC: \"OP\",411,310,\"d|Y\"\n\tC: \"OP\",412,310,\"d|Z\"\n";
    let (clip, _) = only_clip(&anim_doc(&objects, connections));

    assert_eq!(clip.tracks.len(), 2);
    assert!(
        (clip.tracks[0].times[1] - 1.0).abs() < 1e-6,
        "first track ends at {}",
        clip.tracks[0].times[1]
    );
    assert!(
        (clip.duration - 4.0).abs() < 1e-6,
        "duration {} should be the longer track's 4s, not the first's 1s",
        clip.duration
    );
}

#[test]
fn an_unanimated_position_axis_holds_the_models_own_translation() {
    // The reference rig's single position track animates all three axes, so
    // the carry-forward of the model's own translation never runs there. When
    // it is wrong the bone snaps to the origin on the unanimated axes.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t\tProperties70:  {{\n\
         \t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",7,88,9\n\
         \t\t}}\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::clip\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::Base\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::T\", \"\" {{\n\t}}\n{}",
        // Only Y is animated.
        curve(401, &[0, TICK], &[100.0, 200.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Translation\"\n\tC: \"OP\",401,300,\"d|Y\"\n";
    let (clip, _) = only_clip(&anim_doc(&objects, connections));

    let track = &clip.tracks[0];
    assert_eq!(track.kind, TrackKind::Position);
    assert_eq!(track.times.len(), 2);
    // X and Z hold the model's own 7 and 9; Y follows its curve.
    assert_eq!(track.values, vec![7.0, 100.0, 9.0, 7.0, 200.0, 9.0]);
}

#[test]
fn a_curve_hanging_off_something_that_is_not_a_curve_node_is_counted() {
    // Distinct from the 153 curves in the reference rig that have no parent at
    // all: this one IS connected, to an object that cannot hold it. Both lose
    // their keys, so both have to be visible.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::clip\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::Base\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{}{}{}{}",
        curve(400, &[0, TICK], &[0.0, 10.0]),
        curve(401, &[0, TICK], &[0.0, 10.0]),
        curve(402, &[0, TICK], &[0.0, 10.0]),
        // Connected straight to the Model, which is not a curve node.
        curve(500, &[0, TICK], &[0.0, 10.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Rotation\"\n\
         \tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n\
         \tC: \"OP\",500,10,\"d|X\"\n";
    let scene = anim_doc(&objects, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);

    assert_eq!(report.unattached_curves, 1, "the curve on the Model");
    // And the three that ARE attached still produced their track.
    assert_eq!(clips[0].tracks.len(), 1);
    assert_eq!(clips[0].tracks[0].times.len(), 2);
}

#[test]
fn a_rotation_step_of_more_than_half_a_turn_is_subdivided() {
    // Measured: zero steps of 180 degrees or more anywhere in the corpus, so
    // this path is unreachable from real data and the fixture cannot see it.
    // It exists because a quaternion cannot express more than half a turn in
    // one step — without subdivision a 400 degree step would be replayed as
    // the 40 degree remainder, in the wrong direction.
    let (clip, _) = only_clip(&rotating_bone(
        "",
        &[0, TICK],
        [&[0.0, 400.0], &[0.0, 0.0], &[0.0, 0.0]],
    ));
    let track = &clip.tracks[0];

    // ceil(400/180) = 3 sub-steps, plus the key at t=0.
    assert_eq!(track.times.len(), 4, "times {:?}", track.times);
    assert_eq!(track.times[0], 0.0);
    assert!(
        (track.times[3] - 1.0).abs() < 1e-6,
        "the last sub-key must land on the original key time, got {}",
        track.times[3]
    );
    // Strictly increasing: a renderer needs sorted times.
    for pair in track.times.windows(2) {
        assert!(pair[1] > pair[0], "times not increasing: {:?}", track.times);
    }

    // Every consecutive step is now under half a turn, which is the entire
    // point — a dot below zero would mean a step the renderer replays backwards.
    let q: Vec<[f32; 4]> = track
        .values
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    for w in q.windows(2) {
        let dot: f32 = (0..4).map(|i| w[0][i] * w[1][i]).sum();
        assert!(dot > 0.0, "step spans more than half a turn: dot {dot}");
    }

    // And the end really is a 400 degree turn about X: 400 mod 360 = 40.
    let end = q[3];
    let half = (40.0f64 / 2.0).to_radians();
    assert!(
        (f64::from(end[0]) - half.sin()).abs() < 1e-5
            && (f64::from(end[3]) - half.cos()).abs() < 1e-5,
        "final key {end:?} is not a 40 degree rotation about X"
    );
}

#[test]
fn post_rotation_is_applied_as_its_inverse_after_the_key() {
    // No file in the corpus carries a PostRotation, so this is its only
    // coverage. It is applied on the right and inverted; getting either wrong
    // still yields unit quaternions that animate smoothly.
    let props = "\t\t\tP: \"PostRotation\", \"Vector3D\", \"Vector\", \"\",0,0,90\n";
    let keys: [&[f64]; 3] = [&[0.0, 30.0], &[0.0, 0.0], &[0.0, 0.0]];
    let (plain, _) = only_clip(&rotating_bone("", &[0, TICK], keys));
    let (posted, models) = only_clip(&rotating_bone(props, &[0, TICK], keys));

    assert_eq!(
        models.get(10).expect("bone").transform.post_rotation.z,
        90.0,
        "the property was read"
    );

    use m2m_io::fbx::transform::EulerOrder;
    let compose = |d: [f64; 3]| {
        let [a, b, c] = EulerOrder::default().axis_quats(d);
        a * b * c
    };
    let post_inverse = compose([0.0, 0.0, 90.0]).inverse();

    for key in 0..2 {
        let raw = compose([[0.0, 30.0][key], 0.0, 0.0]);
        let want = raw * post_inverse;
        let got = &posted.tracks[0].values[key * 4..key * 4 + 4];
        let dev = [want.x, want.y, want.z, want.w]
            .iter()
            .zip(got.iter())
            .map(|(a, b)| (a - f64::from(*b)).abs())
            .fold(0.0f64, f64::max);
        assert!(dev < 1e-6, "key {key} deviates by {dev}");
    }
    // And it genuinely changed the track.
    let changed = plain.tracks[0]
        .values
        .iter()
        .zip(posted.tracks[0].values.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(changed > 0.1, "PostRotation changed nothing ({changed})");
}

#[test]
fn a_scale_channel_becomes_a_scale_track() {
    // Nothing in the corpus animates scale.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t\tProperties70:  {{\n\
         \t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",2,3,4\n\t\t}}\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::clip\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::Base\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::S\", \"\" {{\n\t}}\n{}",
        curve(400, &[0, TICK], &[1.0, 5.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Scaling\"\n\tC: \"OP\",400,300,\"d|X\"\n";
    let (clip, _) = only_clip(&anim_doc(&objects, connections));

    let track = &clip.tracks[0];
    assert_eq!(track.kind, TrackKind::Scale);
    // X follows its curve; Y and Z hold the model's own 3 and 4.
    assert_eq!(track.values, vec![1.0, 3.0, 4.0, 5.0, 3.0, 4.0]);
}

#[test]
fn malformed_animation_documents_neither_panic_nor_lose_a_clip() {
    // The trust boundary. Each of these arrives as bytes from a file.
    let three = |id: i64| {
        format!(
            "{}{}{}",
            curve(id, &[0, TICK], &[0.0, 10.0]),
            curve(id + 1, &[0, TICK], &[0.0, 10.0]),
            curve(id + 2, &[0, TICK], &[0.0, 10.0])
        )
    };
    let cases: Vec<(&str, Scene)> = vec![
        ("no animation objects at all", anim_doc("", "")),
        (
            "a stack with no layer",
            anim_doc("\tAnimationStack: 100, \"AnimStack::c\", \"\" {\n\t}\n", ""),
        ),
        (
            "a layer with no curve nodes",
            anim_doc(
                "\tAnimationStack: 100, \"AnimStack::c\", \"\" {\n\t}\n\tAnimationLayer: 200, \"AnimLayer::B\", \"\" {\n\t}\n",
                "\tC: \"OO\",200,100\n",
            ),
        ),
        (
            "a curve node driving no model",
            anim_doc(
                &format!("\tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{}", three(400)),
                "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n",
            ),
        ),
        (
            "a curve node whose model does not exist",
            anim_doc(
                &format!("\tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{}", three(400)),
                "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\tC: \"OP\",300,999,\"Lcl Rotation\"\n\tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n",
            ),
        ),
    ];

    for (what, scene) in cases {
        let models = model::parse_all(&scene);
        let (clips, report) = animation::parse_all(&scene, &models);

        // Each malformed shape must also SAY what it lost. Without this the
        // counters are only ever asserted at zero, and a change that stopped
        // incrementing them would pass every test here.
        let expected = match what {
            "a curve node driving no model" => (1, 0),
            "a curve node whose model does not exist" => (0, 1),
            _ => (0, 0),
        };
        assert_eq!(
            (
                report.curve_nodes_without_model,
                report.curve_nodes_with_missing_model
            ),
            expected,
            "{what}: report"
        );
        for clip in &clips {
            assert!(clip.duration.is_finite(), "{what}: duration is not finite");
            assert!(clip.duration >= 0.0, "{what}: negative duration");
            for track in &clip.tracks {
                assert_eq!(
                    track.values.len(),
                    track.times.len() * track.kind.stride(),
                    "{what}: track stride"
                );
                assert!(
                    track.times.iter().all(|t| t.is_finite()),
                    "{what}: non-finite time"
                );
            }
        }
    }

    // A curve with more times than values must be truncated, not indexed past
    // the end while sampling.
    let ragged = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n\
         \tAnimationCurve: 400, \"AnimCurve::\", \"\" {{\n\t\tKeyTime: *4 {{\n\t\t\ta: 0,{TICK},{}, {}\n\t\t}}\n\
         \t\tKeyValueFloat: *2 {{\n\t\t\ta: 0,10\n\t\t}}\n\t}}\n{}",
        2 * TICK,
        3 * TICK,
        curve(401, &[0, TICK], &[0.0, 10.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Rotation\"\n\tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n";
    let scene = anim_doc(&ragged, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);
    assert_eq!(report.mismatched_curve_arrays, 1, "the ragged curve");
    // Truncated to two keys, so the merged array is those two times only.
    assert_eq!(clips[0].tracks[0].times.len(), 2);
}

#[test]
fn a_morph_channel_is_reported_rather_than_silently_dropped() {
    // Morph tracks need the geometry's blend-shape list, which nothing in this
    // project produces yet. Dropping them quietly would let a file whose
    // facial animation vanished load looking complete.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::DeformPercent\", \"\" {{\n\t}}\n{}",
        curve(400, &[0, TICK], &[0.0, 100.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",400,300,\"d|DeformPercent\"\n";
    let scene = anim_doc(&objects, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);

    assert_eq!(report.morph_channels_skipped, 1);
    assert!(clips[0].tracks.is_empty(), "no track is produced");
    // The reference rig has none, so the count stays honest on real files.
    let (_, _, real) = parsed();
    assert_eq!(real.morph_channels_skipped, 0);
}

#[test]
fn a_curve_node_with_no_curves_at_all_is_reported_and_produces_no_track() {
    // The legacy guards the same condition before building a track. A node
    // with no axes would otherwise yield a track with zero keys, which reads
    // downstream as "this bone is pinned at the origin" rather than "this
    // bone was never animated".
    let objects = "\tModel: 10, \"Model::bone\", \"LimbNode\" {\n\t}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {\n\t}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {\n\t}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {\n\t}\n";
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Rotation\"\n";
    let scene = anim_doc(objects, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);

    assert_eq!(report.empty_channels, 1);
    assert!(clips[0].tracks.is_empty(), "no track for an empty channel");
    assert_eq!(clips[0].duration, 0.0);
    // And the reference rig has none, so this stays honest on real files.
    let (_, _, real) = parsed();
    assert_eq!(real.empty_channels, 0);
}

#[test]
fn key_times_land_on_exact_thirtieths_of_a_second() {
    // Independent of the fixture, and the strongest available check on the FBX
    // time constant: Mixamo exports at 30fps, so if 46186158000 were wrong by
    // any factor the spacing would not come out at 1/30 to within f32.
    //
    // A wrong constant is otherwise invisible — every track still has the
    // right shape, the right key count and smooth motion, just played at the
    // wrong speed.
    //
    // This test catches an added or dropped digit. It does NOT catch a small
    // slip like 46_186_160_000, because that shifts a 1/30 gap by only 1.3e-6
    // and these are f32 times; `every_clip_and_key_matches_the_legacy` is what
    // catches that, by requiring the times to agree to 1e-9 with the legacy's.
    let (clips, models, _) = parsed();
    let main = &clips[0];

    let step = 1.0f64 / 30.0;
    for track in &main.tracks {
        assert!(track.times.len() > 1, "single-key track");
        for pair in track.times.windows(2) {
            let dt = f64::from(pair[1]) - f64::from(pair[0]);
            assert!(
                (dt - step).abs() < 1e-6,
                "gap of {dt}s is not 1/30 in {}",
                models.get(track.model).expect("model").name
            );
        }
    }
    // 148 keys at 30fps is 4.9s, which is the clip duration.
    assert_eq!(main.tracks[0].times.len(), 148);
    assert!(
        (main.duration - 147.0 / 30.0).abs() < 1e-6,
        "{}",
        main.duration
    );
}

#[test]
fn the_animated_hips_stay_within_a_human_range() {
    // A second independent check: the fixture pins these values exactly, but a
    // fixture regenerated from a broken loader would pin broken values. Real
    // motion has to stay inside a body.
    let (clips, models, _) = parsed();
    let hips = clips[0]
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Position)
        .expect("position track");
    assert_eq!(
        models.get(hips.model).expect("model").name,
        "mixamorig:Hips"
    );

    let axis = |k: usize| -> (f32, f32) {
        let v: Vec<f32> = hips.values.iter().skip(k).step_by(3).copied().collect();
        (
            v.iter().copied().fold(f32::MAX, f32::min),
            v.iter().copied().fold(f32::MIN, f32::max),
        )
    };
    let (_, _) = axis(0);
    let (min_y, max_y) = axis(1);

    // Measured 39.5..102.8 cm — the clip crouches deeply but never leaves the
    // body's range. Zero or a metre off the floor would mean a dropped or
    // doubled transform.
    assert!(
        (20.0..40.5).contains(&min_y),
        "lowest hip height {min_y} cm is not a pose a body reaches"
    );
    assert!(
        (95.0..115.0).contains(&max_y),
        "highest hip height {max_y} cm is not a standing hip"
    );
    // And it actually moves: a constant track would satisfy the bounds above.
    assert!(
        max_y - min_y > 30.0,
        "the hips barely move ({})",
        max_y - min_y
    );
}

#[test]
fn an_absurd_rotation_step_is_bounded_rather_than_allocating_without_limit() {
    // The subdivision count comes from a degree delta read straight out of the
    // file, so it is attacker-controlled: 1e7 degrees asks for 55,557 keys and
    // 1e12 would ask for billions. Worse, a float-to-int cast in Rust
    // saturates, so an infinite span would give usize::MAX and a loop that
    // never ends.
    let (clip, _) = only_clip(&rotating_bone(
        "",
        &[0, TICK],
        [&[0.0, 1.0e7], &[0.0, 0.0], &[0.0, 0.0]],
    ));
    let track = &clip.tracks[0];
    assert!(
        track.times.len() <= 1025,
        "{} keys from a single step",
        track.times.len()
    );
    assert!(track.times.iter().all(|t| t.is_finite()));
    for pair in track.times.windows(2) {
        assert!(pair[1] > pair[0], "times must stay ordered");
    }
}

#[test]
fn a_non_finite_key_is_dropped_and_counted_instead_of_poisoning_the_track() {
    // 1e400 parses to infinity. Reaching euler_quat it would give a NaN
    // quaternion, which spreads into every vertex the bone touches and
    // surfaces as the mesh vanishing, far from the cause. The legacy guards
    // only mid-track and emits key 0 unconditionally.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n\
         \tAnimationCurve: 400, \"AnimCurve::\", \"\" {{\n\t\tKeyTime: *2 {{\n\t\t\ta: 0,{TICK}\n\t\t}}\n\
         \t\tKeyValueFloat: *2 {{\n\t\t\ta: 1e400,30\n\t\t}}\n\t}}\n{}{}",
        curve(401, &[0, TICK], &[0.0, 0.0]),
        curve(402, &[0, TICK], &[0.0, 0.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Rotation\"\n\
         \tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n";
    let scene = anim_doc(&objects, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);

    assert_eq!(report.non_finite_keys, 1, "the infinite first key");
    let track = &clips[0].tracks[0];
    assert_eq!(track.times.len(), 1, "only the good key survives");
    assert!(
        track.values.iter().all(|v| v.is_finite()),
        "NaN reached the track: {:?}",
        track.values
    );

    // Every key bad means no track at all, rather than an empty or NaN one.
    let (clip, _) = only_clip(&rotating_bone(
        "",
        &[0, TICK],
        [&[f64::NAN, f64::NAN], &[0.0, 0.0], &[0.0, 0.0]],
    ));
    assert!(
        clip.tracks.is_empty(),
        "a wholly unusable channel yields no track"
    );
}

#[test]
fn a_curve_with_a_duplicated_key_time_is_not_taken_at_face_value() {
    // The shortcut in `synchronise` used to compare only key COUNTS. A curve
    // with a duplicated KeyTime has more values than distinct times, so it can
    // match the deduplicated merged length while its keys sit elsewhere —
    // every value then lands one slot late and the last is invented, in a
    // track that looks entirely complete. The legacy still has this.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{}{}{}",
        // X: times 0,0,1,2 -> only three distinct, but four values.
        curve(400, &[0, 0, TICK, 2 * TICK], &[10.0, 20.0, 30.0, 40.0]),
        // Y supplies a fourth distinct time, so the merged array is 0,1,2,3.
        curve(401, &[0, TICK, 2 * TICK, 3 * TICK], &[0.0, 0.0, 0.0, 0.0]),
        curve(402, &[0, TICK, 2 * TICK, 3 * TICK], &[0.0, 0.0, 0.0, 0.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Rotation\"\n\
         \tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n";
    let (clip, _) = only_clip(&anim_doc(&objects, connections));
    let track = &clip.tracks[0];
    assert_eq!(track.times.len(), 4, "merged times 0,1,2,3");

    // Recover the X angle at each key from the quaternion. Taking the values
    // at face value would give 10/20/30/40 — every key after the first shifted
    // a slot early, and 40 landing at t=3 where the curve ends at t=2.
    // Sampling gives 10/30/40/40: t=0 clamps to the first of the duplicate
    // pair, t=1 and t=2 hit real keys, and t=3 clamps past the end.
    let angle_at = |i: usize| {
        let v = &track.values[i * 4..i * 4 + 4];
        2.0 * f64::from(v[0]).atan2(f64::from(v[3])).to_degrees()
    };
    let got: Vec<i64> = (0..4).map(|i| angle_at(i).round() as i64).collect();
    assert_ne!(got, vec![10, 20, 30, 40], "values were taken at face value");
    assert_eq!(
        got,
        vec![10, 30, 40, 40],
        "sampled from the curve's own times"
    );

    // A curve whose times DO match still takes the shortcut and is unchanged.
    let (plain, _) = only_clip(&rotating_bone(
        "",
        &[0, TICK],
        [&[10.0, 40.0], &[0.0, 0.0], &[0.0, 0.0]],
    ));
    assert_eq!(plain.tracks[0].times.len(), 2);
}

#[test]
fn only_the_first_layer_of_a_stack_is_applied_and_the_rest_are_counted() {
    // FBX lets a stack blend several layers with per-layer weights. Applying
    // them all would emit two tracks for the same bone and property in one
    // clip, which a player either double-applies or resolves arbitrarily. The
    // legacy takes children[0]; this matches it and says what it skipped.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::Base\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 201, \"AnimLayer::Override\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 310, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{}{}{}{}{}{}",
        curve(400, &[0, TICK], &[0.0, 10.0]),
        curve(401, &[0, TICK], &[0.0, 10.0]),
        curve(402, &[0, TICK], &[0.0, 10.0]),
        curve(410, &[0, TICK], &[0.0, 90.0]),
        curve(411, &[0, TICK], &[0.0, 90.0]),
        curve(412, &[0, TICK], &[0.0, 90.0])
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",201,100\n\
         \tC: \"OO\",300,200\n\tC: \"OP\",300,10,\"Lcl Rotation\"\n\
         \tC: \"OO\",310,201\n\tC: \"OP\",310,10,\"Lcl Rotation\"\n\
         \tC: \"OP\",400,300,\"d|X\"\n\tC: \"OP\",401,300,\"d|Y\"\n\tC: \"OP\",402,300,\"d|Z\"\n\
         \tC: \"OP\",410,310,\"d|X\"\n\tC: \"OP\",411,310,\"d|Y\"\n\tC: \"OP\",412,310,\"d|Z\"\n";
    let scene = anim_doc(&objects, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);

    assert_eq!(report.extra_layers_skipped, 1, "the override layer");
    assert_eq!(clips[0].tracks.len(), 1, "one track, not one per layer");

    // The reference rig has one layer per stack, so this stays honest there.
    let (_, _, real) = parsed();
    assert_eq!(real.extra_layers_skipped, 0);
}

#[test]
fn a_non_finite_position_value_holds_the_last_good_one() {
    // The rotation path drops bad keys entirely; a vector track cannot, since
    // each axis carries forward independently. An infinite translation would
    // put the bone nowhere and take the mesh with it, so the last good value
    // is held instead — and the loss is counted either way.
    let objects = format!(
        "\tModel: 10, \"Model::bone\", \"LimbNode\" {{\n\t\tProperties70:  {{\n\
         \t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",1,2,3\n\t\t}}\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::T\", \"\" {{\n\t}}\n\
         \tAnimationCurve: 400, \"AnimCurve::\", \"\" {{\n\t\tKeyTime: *3 {{\n\t\t\ta: 0,{TICK},{}\n\t\t}}\n\
         \t\tKeyValueFloat: *3 {{\n\t\t\ta: 50,1e400,70\n\t\t}}\n\t}}\n",
        2 * TICK
    );
    let connections = "\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Translation\"\n\tC: \"OP\",400,300,\"d|X\"\n";
    let scene = anim_doc(&objects, connections);
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);

    assert_eq!(report.non_finite_keys, 1, "the infinite X value");
    let track = &clips[0].tracks[0];
    assert!(
        track.values.iter().all(|v| v.is_finite()),
        "non-finite reached the track: {:?}",
        track.values
    );
    // X holds 50 through the bad key, then takes 70; Y and Z stay at the
    // model's own 2 and 3 throughout.
    assert_eq!(
        track.values,
        vec![50.0, 2.0, 3.0, 50.0, 2.0, 3.0, 70.0, 2.0, 3.0]
    );
}
