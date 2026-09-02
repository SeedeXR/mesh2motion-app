//! Authors a gentle idle animation library for a skeleton-only template glb.
//!
//! A fit template ships with an animation library the app retargets onto the
//! fitted mesh. A creature whose source rig carried no animation (e.g. the
//! giraffe, whose donor mesh was a static pose) still needs one clip so it is a
//! first-class choice. This writes a breathing/sway idle: small sinusoidal
//! rotations on the neck, spine and tail, keyed densely enough to read smoothly.
//! The result is a `<creature>-animations.glb` with one `idle` clip on the
//! template's own bone names, which `retarget_clip` maps by name.
//!
//!     cargo run -p m2m-io --release --example idle_library -- <skeleton.glb> <out.glb>
//!
//! Bones are chosen by name: any `neck*` sways, any `spine*` breathes, any
//! `tail*` flicks. A skeleton with none of those still writes a valid, empty-
//! motion clip rather than failing.

use m2m_io::glb::{self, Channel, Clip, Path};

/// One driven bone: a name, the local axis to rotate about, the amplitude in
/// radians, and the period in seconds.
struct Motion {
    axis: glam::Vec3,
    amp: f32,
    period: f32,
    phase: f32,
}

fn motion_for(name: &str, index_in_chain: usize) -> Option<Motion> {
    // Deeper joints in a chain move a little more; the base barely moves.
    let depth = index_in_chain as f32;
    if name.starts_with("neck") {
        Some(Motion {
            axis: glam::Vec3::Z, // side-to-side sway
            amp: (2.0 + depth * 0.8).to_radians(),
            period: 5.0,
            phase: 0.0,
        })
    } else if name.starts_with("spine") {
        Some(Motion {
            axis: glam::Vec3::X, // breathing nod
            amp: 1.2_f32.to_radians(),
            period: 3.5,
            phase: 0.0,
        })
    } else if name.starts_with("tail") {
        Some(Motion {
            axis: glam::Vec3::Z, // flick
            amp: (4.0 + depth * 2.0).to_radians(),
            period: 2.5,
            phase: 0.5,
        })
    } else {
        None
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: idle_library <skeleton.glb> <out.glb>");
        std::process::exit(2);
    };

    let mut doc = glb::read(&std::fs::read(&input).unwrap()).unwrap();
    let skin = doc.skins.first().expect("skeleton has a skin").clone();

    // Chain depth per bone: how many same-prefix ancestors it has, so deeper
    // neck/tail joints sway more than the base.
    let depth_of = |node: usize| -> usize {
        let prefix = |n: usize| {
            doc.nodes[n]
                .name
                .trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
                .to_owned()
        };
        let mine = prefix(node);
        let mut d = 0;
        let mut cur = doc.nodes[node].parent;
        while let Some(p) = cur {
            if prefix(p) == mine {
                d += 1;
            }
            cur = doc.nodes[p].parent;
        }
        d
    };

    let duration = 5.0_f32;
    let samples = 60usize;
    let times: Vec<f32> = (0..=samples)
        .map(|i| duration * i as f32 / samples as f32)
        .collect();

    let mut channels = Vec::new();
    for &node in &skin.joints {
        let name = doc.nodes[node].name.clone();
        let Some(m) = motion_for(&name, depth_of(node)) else {
            continue;
        };
        let rest = glam::Quat::from_array(doc.nodes[node].transform.rotation);
        let mut values = Vec::with_capacity(times.len() * 4);
        for &t in &times {
            let angle = m.amp * (std::f32::consts::TAU * (t / m.period + m.phase)).sin();
            let local = rest * glam::Quat::from_axis_angle(m.axis, angle);
            values.extend_from_slice(&local.to_array());
        }
        channels.push(Channel {
            node,
            path: Path::Rotation,
            times: times.clone(),
            values,
        });
    }

    let driven = channels.len();
    doc.clips = vec![Clip {
        name: "idle".to_owned(),
        duration,
        channels,
    }];
    let bytes = glb::write(&doc).unwrap();
    std::fs::write(&output, &bytes).unwrap();
    println!(
        "idle: {driven} bones driven, wrote {} bytes to {output}",
        bytes.len()
    );
}
